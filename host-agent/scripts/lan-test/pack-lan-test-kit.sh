#!/usr/bin/env bash
# Build the LAN hardware test kits (Linux + Windows) as two zip files.
#
#   ./pack-lan-test-kit.sh                      # digest from guest-images output
#   ./pack-lan-test-kit.sh --digest <sha256>    # digest by hand
#   ./pack-lan-test-kit.sh --image rootfs.qcow2 # also copy the disk into the kits
#   ./pack-lan-test-kit.sh --qcow2-url https://.../rootfs.qcow2
#
# Output: host-agent/dist-lan-test/dcl-lan-test-{linux,windows}.zip
set -euo pipefail
umask 022

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd -- "$HERE/../../.." && pwd)"
OUT="${LAN_TEST_KIT_DIR:-"$REPO/host-agent/dist-lan-test"}"
SNIPPET="$REPO/guest-images/output/standard/env.snippet"

DIGEST=""
IMAGE=""
QCOW2_URL=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --digest) DIGEST="${2:-}"; shift 2 ;;
    --image) IMAGE="${2:-}"; shift 2 ;;
    --qcow2-url) QCOW2_URL="${2:-}"; shift 2 ;;
    -h|--help) sed -n 2,9p "$0"; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "$DIGEST" && -f "$SNIPPET" ]]; then
  DIGEST="$(sed -n 's/^IMAGE_STANDARD_DIGEST=//p' "$SNIPPET" | head -n1)"
fi
[[ "$DIGEST" =~ ^[0-9a-fA-F]{64}$ ]] || {
  echo "No Standard image digest. Build the guest (guest-images/scripts/build.sh standard)" >&2
  echo "or pass --digest <64 hex chars>." >&2
  exit 1
}
if [[ -n "$IMAGE" ]]; then
  [[ -f "$IMAGE" ]] || { echo "Image does not exist: $IMAGE" >&2; exit 1; }
  case "$IMAGE" in
    *.qcow2|*.img) ;;
    *) echo "Image must end in .qcow2 or .img" >&2; exit 1 ;;
  esac
fi
if [[ -n "$QCOW2_URL" && ! "$QCOW2_URL" =~ ^https?://[^[:space:]]+$ ]]; then
  echo "--qcow2-url must be an http(s) URL." >&2
  exit 1
fi
command -v zip >/dev/null || { echo "zip is required." >&2; exit 1; }

rm -rf "$OUT"
mkdir -p "$OUT"

write_env() {
  {
    echo "IMAGE_STANDARD_DIGEST=$DIGEST"
    [[ -n "$QCOW2_URL" ]] && echo "IMAGE_QCOW2_URL=$QCOW2_URL"
    echo "# LAN_TEST_PORT=8443"
    echo "# LAN_TEST_VCPUS=2"
    echo "# LAN_TEST_MEMORY_GIB=4"
  } >"$1"
}

for os in linux windows; do
  dir="$OUT/dcl-lan-test-$os"
  mkdir -p "$dir"
  cp "$HERE/README-FRIEND.txt" "$dir/README.txt"
  write_env "$dir/lan-test.env"
  if [[ "$os" == linux ]]; then
    install -m 0755 "$HERE/lan-test.sh" "$dir/lan-test.sh"
  else
    cp "$HERE/lan-test.ps1" "$HERE/lan-test.cmd" "$dir/"
    # Windows reads lan-test.env fine with LF; README gets CRLF for Notepad.
    sed 's/$/\r/' "$HERE/README-FRIEND.txt" >"$dir/README.txt"
  fi
  if [[ -n "$IMAGE" ]]; then
    cp "$IMAGE" "$dir/rootfs.${IMAGE##*.}"
  fi
  (cd "$OUT" && zip -qr "dcl-lan-test-$os.zip" "dcl-lan-test-$os")
done

echo "LAN test kits:"
ls -lh "$OUT"/*.zip
if [[ -z "$IMAGE" && -z "$QCOW2_URL" ]]; then
  echo
  echo "No guest disk in the kits. Send rootfs.qcow2 separately (USB/link);"
  echo "your friend puts it next to lan-test.sh / lan-test.cmd."
fi
