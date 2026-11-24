#!/usr/bin/env bash
# Build a signed Standard (Kasm) or Pro (gaming: Sunshine + DCL Stream) guest disk.
# Does not start a guest or a live session; the Host Agent boots the result.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'EOF'
Usage: ./scripts/build.sh standard|pro

Operator inputs (minimal):
  IMAGE_SIGNING_PRIVATE_KEY  Path to a *dev* Ed25519 PEM (never commit).
                             openssl genpkey -algorithm ED25519 -out platform-ed25519.pem
  UBUNTU_CLOUDIMG            Optional local Ubuntu 24.04 cloud image path.
                             If unset, Packer downloads the pinned URL in versions.env.
  PACKER_ACCELERATOR         kvm (Linux default), hvf (macOS default), whpx, none.
                             "none" is TCG — fine for *building*, not for hosting.
  PACKER_MEMORY_MB           Packer VM RAM (default 8192). Use 4096 on a 8 GiB builder.
  PACKER_CPUS                Packer VM vCPUs (default 4). Use 2 on a 2 vCPU builder.

Optional:
  UBUNTU_CLOUDIMG_URL / UBUNTU_CLOUDIMG_SHA256  override versions.env
  KASM_VNC_DEB_URL / KASM_VNC_DEB_SHA256
      Override the pinned Standard KasmVNC package.
  SUNSHINE_DEB_URL  override the official latest Ubuntu 24.04 .deb
  NVIDIA_DRIVER_PACKAGE  Pro: e.g. nvidia-driver-580 (default: newest in the archive)
  RUST_TOOLCHAIN    Pro: Rust used to build dcl-stream-server in the image

Pro image = Standard base + Sunshine + gaming stack (NVIDIA/Mesa drivers,
Steam with Proton, Wine, Lutris, GameMode, MangoHud, virtual monitor and
sound card) + dcl-stream-server built from ../stream. Build it on a Linux
PC with KVM: PACKER_ACCELERATOR=kvm ./scripts/build.sh pro

Outputs (under output/<tier>/, not in git):
  rootfs.qcow2           QEMU/WHPX/VFIO disk
  rootfs.img             raw disk the Host Agent boots (qemu -drive if=virtio)
  rootfs.img.sha256      GNU checksum line
  rootfs.img.sig         standard base64 Ed25519 over the raw 32-byte digest
  rootfs.img.manifest.json
  env.snippet            IMAGE_* lines to copy into Matchmaker / Host Agent .env

Host Agent fetch: point HOST_AGENT_STANDARD_IMAGE_URL / HOST_AGENT_PRO_IMAGE_URL
at rootfs.img (or a URL you host). Copy digest+signature from the manifest.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

LAB_TIER="${1:-}"
if [[ "$LAB_TIER" != "standard" && "$LAB_TIER" != "pro" ]]; then
  usage >&2
  exit 2
fi

if [[ -f "$ROOT/versions.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$ROOT/versions.env"
  set +a
fi

command -v packer >/dev/null || {
  echo "packer is required (https://developer.hashicorp.com/packer/install)" >&2
  exit 1
}
command -v qemu-img >/dev/null || {
  echo "qemu-img is required to emit the raw rootfs Host Agent fetches" >&2
  exit 1
}
command -v openssl >/dev/null || {
  echo "openssl is required to sign (Ed25519)" >&2
  exit 1
}

: "${IMAGE_SIGNING_PRIVATE_KEY:?IMAGE_SIGNING_PRIVATE_KEY is required (Ed25519 PEM path)}"
if [[ ! -f "$IMAGE_SIGNING_PRIVATE_KEY" ]]; then
  echo "signing key not found: $IMAGE_SIGNING_PRIVATE_KEY" >&2
  exit 1
fi

uname_s="$(uname -s)"
case "$uname_s" in
  Linux) DEFAULT_ACCEL=kvm ;;
  Darwin) DEFAULT_ACCEL=hvf ;;
  MINGW*|MSYS*|CYGWIN*) DEFAULT_ACCEL=whpx ;;
  *) DEFAULT_ACCEL=none ;;
esac
ACCEL="${PACKER_ACCELERATOR:-$DEFAULT_ACCEL}"

if [[ -n "${UBUNTU_CLOUDIMG:-}" ]]; then
  CLOUD_URL="$UBUNTU_CLOUDIMG"
else
  CLOUD_URL="${UBUNTU_CLOUDIMG_URL:?pinned UBUNTU_CLOUDIMG_URL missing}"
fi
CLOUD_SHA="${UBUNTU_CLOUDIMG_SHA256:?pinned UBUNTU_CLOUDIMG_SHA256 missing}"

SSH_DIR="$ROOT/.packer-ssh"
mkdir -p "$SSH_DIR"
if [[ ! -f "$SSH_DIR/id_ed25519" ]]; then
  ssh-keygen -t ed25519 -N "" -f "$SSH_DIR/id_ed25519" -C "dcl-packer-throwaway"
fi
SSH_PUB="$(cat "$SSH_DIR/id_ed25519.pub")"

OUT="$ROOT/output/$LAB_TIER"
rm -rf "$OUT"
mkdir -p "$(dirname "$OUT")"

# Pro: a clean copy of the stream/ workspace (no build output) for the
# image to compile dcl-stream-server against its own FFmpeg.
STREAM_SRC="$ROOT/.stream-src"
rm -rf "$STREAM_SRC"
if [[ "$LAB_TIER" == "pro" ]]; then
  mkdir -p "$STREAM_SRC"
  (cd "$ROOT/../stream" && tar --exclude=./target --exclude=./target-linux -cf - .) | tar -xf - -C "$STREAM_SRC"
fi

export PACKER_LOG="${PACKER_LOG:-0}"
packer init "$ROOT/packer"
packer build \
  -var "lab_tier=$LAB_TIER" \
  -var "cloud_image_url=$CLOUD_URL" \
  -var "cloud_image_checksum=$CLOUD_SHA" \
  -var "ssh_public_key=$SSH_PUB" \
  -var "ssh_private_key_file=$SSH_DIR/id_ed25519" \
  -var "output_dir=$OUT" \
  -var "accelerator=$ACCEL" \
  -var "memory_mb=${PACKER_MEMORY_MB:-8192}" \
  -var "cpus=${PACKER_CPUS:-4}" \
  -var "kasm_vnc_deb_url=${KASM_VNC_DEB_URL:-}" \
  -var "kasm_vnc_deb_sha256=${KASM_VNC_DEB_SHA256:-}" \
  -var "sunshine_deb_url=${SUNSHINE_DEB_URL:-}" \
  -var "nvidia_driver_package=${NVIDIA_DRIVER_PACKAGE:-}" \
  -var "rust_toolchain=${RUST_TOOLCHAIN:?pinned RUST_TOOLCHAIN missing}" \
  -var "stream_source_dir=$STREAM_SRC" \
  "$ROOT/packer"

QCOW="$OUT/rootfs.qcow2"
if [[ ! -f "$QCOW" ]]; then
  echo "packer did not emit $QCOW" >&2
  exit 1
fi

qemu-img convert -O raw "$QCOW" "$OUT/rootfs.img"

python3 "$ROOT/scripts/sign_image.py" sign \
  --image "$OUT/rootfs.img" \
  --key "$IMAGE_SIGNING_PRIVATE_KEY" \
  --lab-tier "$LAB_TIER"

echo "signed $OUT/rootfs.img — copy env.snippet into Matchmaker/Host Agent .env" >&2
