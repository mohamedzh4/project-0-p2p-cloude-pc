#!/usr/bin/env bash
# Optional: copy a Firecracker-capable kernel next to a raw rootfs.
# Packer emits a QEMU disk with GRUB. Firecracker does not run GRUB; it needs
# a separate vmlinux/bzImage. This script does not invent one.
#
# Operator supplies FIRECRACKER_KERNEL (a kernel that can mount this Ubuntu
# rootfs: virtio-blk, virtio-net, overlayfs, cgroup v2). The Firecracker
# getting-started demo kernel is *not* enough for Docker/Kasm.
set -euo pipefail

if [[ $# -lt 2 ]]; then
  echo "Usage: $0 ROOTFS.img DEST_VMLINUX" >&2
  echo "Set FIRECRACKER_KERNEL to a kernel file to copy, or extract from the" >&2
  echo "guest on Linux: mount the raw disk and use /usr/src/*/scripts/extract-vmlinux." >&2
  exit 2
fi

ROOTFS="$1"
DEST="$2"

if [[ -n "${FIRECRACKER_KERNEL:-}" ]]; then
  cp "$FIRECRACKER_KERNEL" "$DEST"
  echo "copied $FIRECRACKER_KERNEL -> $DEST"
  exit 0
fi

echo "No FIRECRACKER_KERNEL. Refusing to guess a vmlinux from $ROOTFS." >&2
echo "Extracting /boot/vmlinuz needs a Linux loop mount (not done on macOS CI)." >&2
exit 1
