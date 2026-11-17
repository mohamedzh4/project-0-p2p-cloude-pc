#!/usr/bin/env bash
# Ubuntu 22.04/24.04 Standard host setup (QEMU+KVM).
# Run from an extracted friend kit: bash linux-host-setup.sh
set -euo pipefail
umask 077

if [[ "$(uname -s)" != "Linux" ]] || ! command -v apt-get >/dev/null; then
  echo "This setup supports Ubuntu/Debian Linux hosts only." >&2
  exit 1
fi
if [[ "${EUID}" -eq 0 ]]; then
  echo "Run this as the desktop user; the script uses sudo when needed." >&2
  exit 1
fi

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${DCL_FRIEND_ENV:-"$HERE/friend.env"}"
[[ -f "$ENV_FILE" ]] || {
  echo "Missing friend.env next to this script." >&2
  exit 1
}

allowed_key() {
  case "$1" in
    HOST_BOOTSTRAP_TOKEN|MATCHMAKER_URL|HEADSCALE_LOGIN_SERVER|\
    IMAGE_SIGNING_PUBLIC_KEY|IMAGE_STANDARD_DIGEST|IMAGE_STANDARD_SIGNATURE|\
    HOST_DISPLAY_NAME|HOST_AGENT_LISTEN_ADDR|HOST_AGENT_URL|\
    IMAGE_RAW_URL|IMAGE_QCOW2_URL|TUNNEL_SSH|TUNNEL_REMOTE_PORT) return 0 ;;
    *) return 1 ;;
  esac
}

while IFS='=' read -r key value; do
  key="${key#"${key%%[![:space:]]*}"}"
  key="${key%"${key##*[![:space:]]}"}"
  [[ -z "$key" || "$key" == \#* ]] && continue
  allowed_key "$key" || {
    echo "Unsupported key in friend.env: $key" >&2
    exit 1
  }
  value="${value%$'\r'}"
  value="${value#\"}"
  value="${value%\"}"
  printf -v "$key" '%s' "$value"
done < "$ENV_FILE"

: "${HOST_BOOTSTRAP_TOKEN:?HOST_BOOTSTRAP_TOKEN is required in friend.env}"
: "${MATCHMAKER_URL:?MATCHMAKER_URL is required in friend.env}"
: "${HEADSCALE_LOGIN_SERVER:?HEADSCALE_LOGIN_SERVER is required in friend.env}"
: "${IMAGE_SIGNING_PUBLIC_KEY:?IMAGE_SIGNING_PUBLIC_KEY is required in friend.env}"
: "${IMAGE_STANDARD_DIGEST:?IMAGE_STANDARD_DIGEST is required in friend.env}"
: "${IMAGE_STANDARD_SIGNATURE:?IMAGE_STANDARD_SIGNATURE is required in friend.env}"

[[ "${#HOST_BOOTSTRAP_TOKEN}" -ge 16 ]] || {
  echo "HOST_BOOTSTRAP_TOKEN must be at least 16 characters." >&2
  exit 1
}
[[ "$MATCHMAKER_URL" =~ ^https?://[^[:space:]]+$ ]] || {
  echo "MATCHMAKER_URL must be an http(s) URL." >&2
  exit 1
}
[[ "$HEADSCALE_LOGIN_SERVER" =~ ^https?://[^[:space:]]+$ ]] || {
  echo "HEADSCALE_LOGIN_SERVER must be an http(s) URL." >&2
  exit 1
}
[[ "$IMAGE_SIGNING_PUBLIC_KEY" =~ ^[0-9a-fA-F]{64}$ ]] || {
  echo "IMAGE_SIGNING_PUBLIC_KEY must be 64 hexadecimal characters." >&2
  exit 1
}
[[ "$IMAGE_STANDARD_DIGEST" =~ ^[0-9a-fA-F]{64}$ ]] || {
  echo "IMAGE_STANDARD_DIGEST must be 64 hexadecimal characters." >&2
  exit 1
}

sudo -v
sudo apt-get update
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
  ca-certificates curl qemu-system-x86 qemu-utils

if [[ ! -e /dev/kvm ]]; then
  echo "/dev/kvm is missing. Enable virtualization in firmware and load kvm." >&2
  exit 1
fi
sudo usermod -aG kvm "$USER"

if ! command -v tailscale >/dev/null; then
  curl -fsSL https://tailscale.com/install.sh | sudo sh
fi

IMAGE_DIR="${XDG_CACHE_HOME:-"$HOME/.cache"}/dcl-host-agent/images/standard"
mkdir -p "$IMAGE_DIR"
RAW_IMAGE="$IMAGE_DIR/rootfs.img"

if [[ ! -f "$RAW_IMAGE" ]]; then
  if [[ -n "${IMAGE_RAW_URL:-}" ]]; then
    curl --fail --location --retry 5 --continue-at - \
      --output "$RAW_IMAGE.part" "$IMAGE_RAW_URL"
    mv "$RAW_IMAGE.part" "$RAW_IMAGE"
  elif [[ -f "$HERE/rootfs.img" ]]; then
    cp "$HERE/rootfs.img" "$RAW_IMAGE"
  elif [[ -n "${IMAGE_QCOW2_URL:-}" ]]; then
    curl --fail --location --retry 5 --continue-at - \
      --output "$IMAGE_DIR/rootfs.qcow2.part" "$IMAGE_QCOW2_URL"
    mv "$IMAGE_DIR/rootfs.qcow2.part" "$IMAGE_DIR/rootfs.qcow2"
    qemu-img convert -p -O raw "$IMAGE_DIR/rootfs.qcow2" "$RAW_IMAGE"
    rm -f "$IMAGE_DIR/rootfs.qcow2"
  elif [[ -f "$HERE/rootfs.qcow2" ]]; then
    qemu-img convert -p -O raw "$HERE/rootfs.qcow2" "$RAW_IMAGE"
  else
    echo "Add rootfs.img/rootfs.qcow2 to this kit or set IMAGE_RAW_URL/IMAGE_QCOW2_URL." >&2
    exit 1
  fi
fi

actual_digest="$(sha256sum "$RAW_IMAGE" | awk '{print $1}')"
if [[ "${actual_digest,,}" != "${IMAGE_STANDARD_DIGEST,,}" ]]; then
  rm -f "$RAW_IMAGE"
  echo "Guest image SHA-256 mismatch; deleted the untrusted image." >&2
  exit 1
fi

deb="$(find "$HERE" -maxdepth 1 -type f -name '*.deb' -print -quit)"
[[ -n "$deb" ]] || {
  echo "Host Agent .deb is missing from the kit." >&2
  exit 1
}
sudo apt-get install -y "$deb"

agent_bin="$(command -v host-agent || true)"
[[ -n "$agent_bin" ]] || {
  echo "Host Agent installed, but the host-agent binary was not found." >&2
  exit 1
}

# Default: the Host Agent fetches its commands from the server itself, so
# nothing has to reach this PC from the internet. Only a friend.env that
# sets HOST_AGENT_URL (or the old reverse tunnel) makes the agent listen.
agent_url="${HOST_AGENT_URL:-}"
if [[ -z "$agent_url" && -n "${TUNNEL_SSH:-}" && -n "${TUNNEL_REMOTE_PORT:-}" ]]; then
  agent_url="http://127.0.0.1:${TUNNEL_REMOTE_PORT}"
fi
listen_lines=""
listen_addr=""
if [[ -n "$agent_url" ]]; then
  [[ "$agent_url" =~ ^https?://[^[:space:]]+$ ]] || {
    echo "HOST_AGENT_URL must be an http or https URL." >&2
    exit 1
  }
  listen_addr="${HOST_AGENT_LISTEN_ADDR:-127.0.0.1:7842}"
  listen_lines="HOST_AGENT_LISTEN_ADDR=$listen_addr
HOST_AGENT_URL=$agent_url"
else
  echo "This PC will fetch its commands from the server. No router setup is needed."
fi

APP_DIR="${XDG_CONFIG_HOME:-"$HOME/.config"}/com.distributedcomputelab.hostagent"
mkdir -p "$APP_DIR"
cat > "$APP_DIR/.env" <<EOF
MATCHMAKER_URL=$MATCHMAKER_URL
HOST_BOOTSTRAP_TOKEN=$HOST_BOOTSTRAP_TOKEN
HOST_DISPLAY_NAME=${HOST_DISPLAY_NAME:-$(hostname)}
$listen_lines
HEADSCALE_LOGIN_SERVER=$HEADSCALE_LOGIN_SERVER
IMAGE_SIGNING_PUBLIC_KEY=$IMAGE_SIGNING_PUBLIC_KEY
IMAGE_LOCAL_DIGEST=$IMAGE_STANDARD_DIGEST
IMAGE_STANDARD_DIGEST=$IMAGE_STANDARD_DIGEST
IMAGE_STANDARD_SIGNATURE=$IMAGE_STANDARD_SIGNATURE
HOST_AGENT_STANDARD_IMAGE_URL=file://$RAW_IMAGE
HOST_AGENT_QEMU_BIN=$(command -v qemu-system-x86_64)
EOF
chmod 600 "$APP_DIR/.env"

if [[ -n "${TUNNEL_SSH:-}" && -n "$listen_addr" ]]; then
  key="$HERE/tunnel.pem"
  [[ -f "$key" ]] || {
    echo "TUNNEL_SSH is set but tunnel.pem is missing." >&2
    exit 1
  }
  install -m 0600 "$key" "$APP_DIR/tunnel.pem"
  remote_port="${TUNNEL_REMOTE_PORT:-27842}"
  mkdir -p "$HOME/.config/systemd/user"
  cat > "$HOME/.config/systemd/user/dcl-host-tunnel.service" <<EOF
[Unit]
Description=DCL Host Agent reverse tunnel
After=network-online.target

[Service]
ExecStart=/usr/bin/ssh -N -o BatchMode=yes -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -o StrictHostKeyChecking=accept-new -o IdentitiesOnly=yes -i $APP_DIR/tunnel.pem -R 127.0.0.1:${remote_port}:${listen_addr} $TUNNEL_SSH
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
EOF
  systemctl --user daemon-reload
  systemctl --user enable --now dcl-host-tunnel.service
fi

mkdir -p "$HOME/.config/autostart"
cat > "$HOME/.config/autostart/dcl-host-agent.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=DCL Host Agent
Exec=$agent_bin
Terminal=false
X-GNOME-Autostart-enabled=true
EOF

nohup "$agent_bin" >"$APP_DIR/host-agent.log" 2>&1 &
echo "Linux host setup complete. Open Host Agent, choose resources, enable sharing, then approve the host in Matchmaker."
