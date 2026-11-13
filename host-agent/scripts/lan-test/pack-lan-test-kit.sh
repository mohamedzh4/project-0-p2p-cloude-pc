#!/usr/bin/env bash
# Build the LAN hardware test kits (Linux + Windows) as two zip files.
#
#   ./pack-lan-test-kit.sh                      # quick mode (no image build)
#   ./pack-lan-test-kit.sh --signed             # signed image, digest from
#                                               # guest-images output
#   ./pack-lan-test-kit.sh --digest <sha256>    # signed image, digest by hand
#   ./pack-lan-test-kit.sh --signed --image rootfs.qcow2   # copy disk in
#   ./pack-lan-test-kit.sh --signed --qcow2-url https://.../rootfs.qcow2
#
# Quick mode: the friend's PC downloads the pinned Ubuntu 24.04 cloud image
# and the VM installs XFCE + KasmVNC with guest-images/scripts/provision-kasm.sh
# on first boot (seed.iso in the kit). Needs internet on the friend's PC.
#
# Output: host-agent/dist-lan-test/dcl-lan-test-{linux,windows}.zip
set -euo pipefail
umask 022

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd -- "$HERE/../../.." && pwd)"
OUT="${LAN_TEST_KIT_DIR:-"$REPO/host-agent/dist-lan-test"}"
SNIPPET="$REPO/guest-images/output/standard/env.snippet"
VERSIONS="$REPO/guest-images/versions.env"
PROVISION_KASM="$REPO/guest-images/scripts/provision-kasm.sh"

MODE=quick
DIGEST=""
IMAGE=""
QCOW2_URL=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --signed) MODE=signed; shift ;;
    --digest) MODE=signed; DIGEST="${2:-}"; shift 2 ;;
    --image) IMAGE="${2:-}"; shift 2 ;;
    --qcow2-url) QCOW2_URL="${2:-}"; shift 2 ;;
    -h|--help) sed -n 2,15p "$0"; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done

if [[ "$MODE" == quick && ( -n "$IMAGE" || -n "$QCOW2_URL" ) ]]; then
  echo "--image / --qcow2-url need --signed (quick mode downloads Ubuntu itself)." >&2
  exit 1
fi
if [[ "$MODE" == signed ]]; then
  if [[ -z "$DIGEST" && -f "$SNIPPET" ]]; then
    DIGEST="$(sed -n 's/^IMAGE_STANDARD_DIGEST=//p' "$SNIPPET" | head -n1)"
  fi
  [[ "$DIGEST" =~ ^[0-9a-fA-F]{64}$ ]] || {
    echo "No Standard image digest. Build the guest (guest-images/scripts/build.sh standard)" >&2
    echo "or pass --digest <64 hex chars>. Without a built image, use quick mode." >&2
    exit 1
  }
fi
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

# --- quick-mode seed.iso (cloud-init NoCloud, volume label cidata) ----------
build_seed() {
  local dest="$1" work kasm_url kasm_sha b64
  kasm_url="$(sed -n 's/^KASM_VNC_DEB_URL=//p' "$VERSIONS")"
  kasm_sha="$(sed -n 's/^KASM_VNC_DEB_SHA256=//p' "$VERSIONS")"
  [[ -n "$kasm_url" && "$kasm_sha" =~ ^[0-9a-f]{64}$ ]] || {
    echo "KASM_VNC_DEB_URL / KASM_VNC_DEB_SHA256 missing in $VERSIONS" >&2
    exit 1
  }
  b64="$(base64 <"$PROVISION_KASM" | tr -d '\n')"
  work="$(mktemp -d)"
  sed -e "s|@PROVISION_KASM_B64@|$b64|" \
      -e "s|@KASM_VNC_DEB_URL@|$kasm_url|" \
      -e "s|@KASM_VNC_DEB_SHA256@|$kasm_sha|" \
      "$HERE/quick-user-data.in" >"$work/user-data"
  if grep -q '@[A-Z0-9_]*@' "$work/user-data"; then
    echo "Unfilled placeholder in user-data." >&2
    exit 1
  fi
  printf 'instance-id: dcl-lan-test-quick-1\nlocal-hostname: dcl-lan-test\n' >"$work/meta-data"
  if command -v xorriso >/dev/null; then
    xorriso -as mkisofs -quiet -o "$dest" -V cidata -J -R "$work/user-data" "$work/meta-data"
  elif command -v genisoimage >/dev/null; then
    genisoimage -quiet -o "$dest" -V cidata -J -R "$work/user-data" "$work/meta-data"
  elif command -v mkisofs >/dev/null; then
    mkisofs -quiet -o "$dest" -V cidata -J -R "$work/user-data" "$work/meta-data"
  elif command -v hdiutil >/dev/null; then
    local src="$work/src"
    mkdir -p "$src"
    cp "$work/user-data" "$work/meta-data" "$src/"
    hdiutil makehybrid -quiet -o "$dest" -iso -joliet \
      -default-volume-name cidata "$src"
  else
    echo "Need xorriso, genisoimage, mkisofs or hdiutil to build seed.iso." >&2
    exit 1
  fi
  rm -rf "$work"
}

if [[ "$MODE" == quick ]]; then
  build_seed "$OUT/seed.iso"
fi

write_env() {
  {
    echo "LAN_TEST_MODE=$MODE"
    if [[ "$MODE" == signed ]]; then
      echo "IMAGE_STANDARD_DIGEST=$DIGEST"
      [[ -n "$QCOW2_URL" ]] && echo "IMAGE_QCOW2_URL=$QCOW2_URL"
    fi
    echo "# LAN_TEST_PORT=8443"
    echo "# LAN_TEST_VCPUS=2"
    echo "# LAN_TEST_MEMORY_GIB=4"
    echo "# LAN_TEST_SETUP_MINUTES=45"
    echo "# LAN_TEST_BOOT_MINUTES=10"
  } >"$1"
}

for os in linux windows; do
  dir="$OUT/dcl-lan-test-$os"
  mkdir -p "$dir"
  write_env "$dir/lan-test.env"
  if [[ "$os" == linux ]]; then
    install -m 0755 "$HERE/lan-test.sh" "$dir/lan-test.sh"
    cp "$HERE/README-FRIEND.txt" "$dir/README.txt"
  else
    cp "$HERE/lan-test.ps1" "$HERE/lan-test.cmd" "$dir/"
    # README gets CRLF for Notepad.
    sed 's/$/\r/' "$HERE/README-FRIEND.txt" >"$dir/README.txt"
  fi
  [[ "$MODE" == quick ]] && cp "$OUT/seed.iso" "$dir/seed.iso"
  if [[ -n "$IMAGE" ]]; then
    cp "$IMAGE" "$dir/rootfs.${IMAGE##*.}"
  fi
  (cd "$OUT" && zip -qr "dcl-lan-test-$os.zip" "dcl-lan-test-$os")
done

echo "LAN test kits ($MODE mode):"
ls -lh "$OUT"/*.zip
if [[ "$MODE" == signed && -z "$IMAGE" && -z "$QCOW2_URL" ]]; then
  echo
  echo "No guest disk in the kits. Send rootfs.qcow2 separately (USB/link);"
  echo "your friend puts it next to lan-test.sh / lan-test.cmd."
fi
