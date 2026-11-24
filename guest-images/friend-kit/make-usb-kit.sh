#!/usr/bin/env bash
# Make the Phase 4 USB kit on your Mac (or any machine with the repo).
#
#   bash guest-images/friend-kit/make-usb-kit.sh /Volumes/MYUSB
#
# Writes MYUSB/DCL-PHASE4/ with:
#   phase4.sh               the one command to run on the borrowed PC
#   dcl-guest-images.tgz    the guest image code from THIS checkout
#   key/dcl-dev-ed25519.pem the signing key, kept on the USB, never on the PC
#   HOW-TO.txt
#
# Key: $IMAGE_SIGNING_PRIVATE_KEY, else ~/dcl-dev-ed25519.pem.
# The USB drive must be exFAT (FAT32 cannot hold the 4+ GB disk).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
USB="${1:-}"
KEY="${IMAGE_SIGNING_PRIVATE_KEY:-$HOME/dcl-dev-ed25519.pem}"

[[ -n "$USB" && -d "$USB" ]] || { echo "usage: $0 /Volumes/<USB drive>" >&2; exit 2; }
[[ -f "$KEY" ]] || { echo "signing key not found: $KEY" >&2; exit 1; }
grep -q 'BEGIN PRIVATE KEY' "$KEY" || { echo "$KEY is not a PEM private key" >&2; exit 1; }

if command -v diskutil >/dev/null 2>&1; then
  fs=$( (diskutil info "$USB" 2>/dev/null || true) | awk -F: '/Type \(Bundle\)/ {gsub(/ /,"",$2); print $2}')
  if [[ "$fs" == msdos ]]; then
    echo "The USB drive is FAT32: it cannot hold the 4+ GB disk." >&2
    echo "Reformat it as exFAT in Disk Utility, then run this again." >&2
    exit 1
  fi
fi

DEST="$USB/DCL-PHASE4"
mkdir -p "$DEST/key"
cp "$HERE/phase4.sh" "$DEST/phase4.sh"
cp "$KEY" "$DEST/key/dcl-dev-ed25519.pem"
chmod 600 "$DEST/key/dcl-dev-ed25519.pem" 2>/dev/null || true

# No macOS ._ files or extended attributes inside the archive: GNU tar on
# Linux warns about every com.apple.provenance header otherwise.
mac_flags=()
if tar --version 2>/dev/null | grep -q bsdtar; then
  mac_flags=(--no-mac-metadata --no-xattrs)
fi
COPYFILE_DISABLE=1 tar "${mac_flags[@]}" -C "$REPO" -czf "$DEST/dcl-guest-images.tgz" \
  --exclude 'guest-images/output' --exclude 'guest-images/cache' \
  --exclude 'guest-images/.packer-ssh' --exclude '__pycache__' \
  --exclude 'guest-images/friend-kit' \
  guest-images

cat >"$DEST/HOW-TO.txt" <<'EOF'
On the borrowed Linux PC (Ubuntu or Arch), plugged in to the internet:

1. Plug in this USB drive and open a terminal.
2. Run (replace the path with where the drive is mounted):

     sudo bash /media/$USER/<USB>/DCL-PHASE4/phase4.sh

   On Arch the drive is often under /run/media/$USER/<USB>/.

3. Wait 30-60 minutes. It ends with "PHASE 4 DONE" or "PHASE 4 FAILED".
   Either way it removes everything it installed or created on the PC.
4. Eject the drive. The disk is in DCL-PHASE4/output/.

If the PC lost power during the build, plug the drive in again and run:

     sudo bash /media/$USER/<USB>/DCL-PHASE4/phase4.sh --cleanup-only

The signing key is in DCL-PHASE4/key/. Keep this drive safe; delete the key
folder once Phase 4 is done if you keep your own copy on your Mac.
EOF

sync
echo "Kit ready in $DEST:"
ls -lh "$DEST" "$DEST/key"
