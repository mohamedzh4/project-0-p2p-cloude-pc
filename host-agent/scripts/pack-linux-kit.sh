#!/usr/bin/env bash
# Build a folder for an Ubuntu Standard host. Does not compile the .deb.
set -euo pipefail
umask 077

ROOT="$(cd -- "$(dirname -- "$0")/.." && pwd)"
OUT="${LINUX_KIT_DIR:-"$ROOT/dist-linux"}"
FRIEND_ENV="${FRIEND_ENV_FILE:-"$ROOT/.env.friend"}"
DEB="${HOST_AGENT_DEB:-${1:-}}"
IMAGE="${IMAGE_FILE:-${2:-}}"

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
install -m 0600 "$FRIEND_ENV" "$OUT/friend.env"
cp "$DEB" "$OUT/host-agent.deb"

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
echo "On Ubuntu: bash linux-host-setup.sh"
