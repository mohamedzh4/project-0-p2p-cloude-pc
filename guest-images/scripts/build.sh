#!/usr/bin/env bash
# Build a signed Standard (Kasm) or Pro (Sunshine) guest disk.
# Does not start Firecracker, QEMU+WHPX, or a live session. Host Agent boot is later.
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

Optional:
  UBUNTU_CLOUDIMG_URL / UBUNTU_CLOUDIMG_SHA256  override versions.env
  DCL_KASM_ADMIN_PASSWORD / DCL_KASM_USER_PASSWORD
      Standard only. If unset, build.sh generates them into
      output/standard/kasm-credentials.txt (gitignored).
  SUNSHINE_DEB_URL  override the official latest Ubuntu 24.04 .deb
  FIRECRACKER_KERNEL  Linux only; copied next to rootfs as vmlinux after convert.

Outputs (under output/<tier>/, not in git):
  rootfs.qcow2           QEMU/WHPX/VFIO disk
  rootfs.img             raw (Firecracker virtio-blk / qemu -drive)
  rootfs.img.sha256      GNU checksum line
  rootfs.img.sig         standard base64 Ed25519 over the raw 32-byte digest
  rootfs.img.manifest.json
  env.snippet            IMAGE_* lines to copy into Matchmaker / Host Agent .env
  vmlinux                only if FIRECRACKER_KERNEL is set

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
mkdir -p "$OUT"

if [[ "$LAB_TIER" == "standard" ]]; then
  if [[ -z "${DCL_KASM_ADMIN_PASSWORD:-}" || -z "${DCL_KASM_USER_PASSWORD:-}" ]]; then
    DCL_KASM_ADMIN_PASSWORD="$(openssl rand -base64 18)"
    DCL_KASM_USER_PASSWORD="$(openssl rand -base64 18)"
    umask 077
    cat > "$OUT/kasm-credentials.txt" <<EOF
admin@kasm.local
$DCL_KASM_ADMIN_PASSWORD
user@kasm.local
$DCL_KASM_USER_PASSWORD
EOF
    echo "wrote $OUT/kasm-credentials.txt (gitignored; do not commit)" >&2
  fi
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
  -var "kasm_tarball_url=${KASM_TARBALL_URL:-}" \
  -var "kasm_tarball_sha256=${KASM_TARBALL_SHA256:-}" \
  -var "kasm_admin_password=${DCL_KASM_ADMIN_PASSWORD:-}" \
  -var "kasm_user_password=${DCL_KASM_USER_PASSWORD:-}" \
  -var "sunshine_deb_url=${SUNSHINE_DEB_URL:-}" \
  "$ROOT/packer"

QCOW="$OUT/rootfs.qcow2"
if [[ ! -f "$QCOW" ]]; then
  echo "packer did not emit $QCOW" >&2
  exit 1
fi

qemu-img convert -O raw "$QCOW" "$OUT/rootfs.img"

if [[ -n "${FIRECRACKER_KERNEL:-}" ]]; then
  cp "$FIRECRACKER_KERNEL" "$OUT/vmlinux"
fi

python3 "$ROOT/scripts/sign_image.py" sign \
  --image "$OUT/rootfs.img" \
  --key "$IMAGE_SIGNING_PRIVATE_KEY" \
  --lab-tier "$LAB_TIER"

echo "signed $OUT/rootfs.img — copy env.snippet into Matchmaker/Host Agent .env" >&2
echo "Host Agent still does not boot this disk in phase 26." >&2
