#!/usr/bin/env bash
# Build a folder for an Ubuntu Standard host. Does not compile the .deb.
set -euo pipefail
umask 077

ROOT="$(cd -- "$(dirname -- "$0")/.." && pwd)"
OUT="${LINUX_KIT_DIR:-"$ROOT/dist-linux"}"
FRIEND_ENV="${FRIEND_ENV_FILE:-"$ROOT/.env.friend"}"
DEB="${HOST_AGENT_DEB:-${1:-}}"
IMAGE="${IMAGE_FILE:-${2:-}}"
APPIMAGE="${HOST_AGENT_APPIMAGE:-}"
if [[ -n "$APPIMAGE" && ( ! -f "$APPIMAGE" || "$APPIMAGE" != *.AppImage ) ]]; then
  echo "HOST_AGENT_APPIMAGE must point to an existing .AppImage file." >&2
  exit 1
fi

if [[ ! -f "$FRIEND_ENV" ]]; then
  echo "Missing $FRIEND_ENV (gitignored)." >&2
  exit 1
fi
if [[ -z "$DEB" || ! -f "$DEB" || "$DEB" != *.deb ]]; then
  echo "Usage: ./scripts/pack-linux-kit.sh /path/to/host-agent.deb [/path/to/rootfs.img|rootfs.qcow2]" >&2
  exit 1
fi
if [[ -n "$IMAGE" && ! -f "$IMAGE" ]]; then
  echo "Image file does not exist: $IMAGE" >&2
  exit 1
fi

rm -rf "$OUT"
mkdir -p "$OUT"
install -m 0755 "$ROOT/scripts/linux-host-setup.sh" "$OUT/linux-host-setup.sh"
# Only the keys linux-host-setup.sh accepts: it stops on any other key, and
# .env.friend also holds settings meant for the operator's own agent.
grep -E '^(HOST_BOOTSTRAP_TOKEN|MATCHMAKER_URL|HEADSCALE_LOGIN_SERVER|IMAGE_SIGNING_PUBLIC_KEY|IMAGE_STANDARD_DIGEST|IMAGE_STANDARD_SIGNATURE|HOST_DISPLAY_NAME|HOST_AGENT_LISTEN_ADDR|HOST_AGENT_URL|IMAGE_RAW_URL|IMAGE_QCOW2_URL|TUNNEL_SSH|TUNNEL_REMOTE_PORT)=' \
  "$FRIEND_ENV" >"$OUT/friend.env"
chmod 0600 "$OUT/friend.env"
cp "$DEB" "$OUT/host-agent.deb"
# Arch hosts run the AppImage instead of the .deb.
if [[ -n "$APPIMAGE" ]]; then
  cp "$APPIMAGE" "$OUT/host-agent.AppImage"
  chmod 0755 "$OUT/host-agent.AppImage"
fi

if [[ -f "$ROOT/.tunnel/dcl-tunnel.pem" ]]; then
  install -m 0600 "$ROOT/.tunnel/dcl-tunnel.pem" "$OUT/tunnel.pem"
fi
if [[ -n "$IMAGE" ]]; then
  case "$IMAGE" in
    *.img) cp "$IMAGE" "$OUT/rootfs.img" ;;
    *.qcow2) cp "$IMAGE" "$OUT/rootfs.qcow2" ;;
    *)
      echo "Image must end in .img or .qcow2." >&2
      exit 1
      ;;
  esac
fi

echo "Linux host kit ready at $OUT"
echo "On Ubuntu/Debian or Arch: bash linux-host-setup.sh"
