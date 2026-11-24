#!/usr/bin/env bash
# Boot the Pro gaming image on this Linux PC with its GPU passed through, and
# stream it over the LAN with DCL Stream. No Matchmaker, mesh or Host Agent:
# the session token goes in through SMBIOS, the stream port is forwarded.
#
#   ./scripts/run-pro-lan-test.sh output/pro/rootfs.img 0000:01:00.0 [0000:01:00.1]
#
# The GPU (and its HDMI audio function, if you pass it) must already be bound
# to vfio-pci; scripts/../../host-agent/scripts/pro-gpu-check.sh tells you.
# Runbook: docs/runbooks/stream-lan-test.md
set -euo pipefail

usage() {
  sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
}
[[ $# -ge 2 ]] || usage
IMAGE=$1
shift
GPUS=("$@")

PORT=${DCL_STREAM_PORT:-47100}
MEM=${DCL_GUEST_MEMORY:-16G}
CPUS=${DCL_GUEST_CPUS:-8}

[[ -f "$IMAGE" ]] || { echo "image not found: $IMAGE" >&2; exit 1; }
[[ -w /dev/kvm ]] || { echo "/dev/kvm is not usable (KVM off in BIOS, or not in the kvm group)" >&2; exit 1; }
command -v qemu-system-x86_64 >/dev/null || { echo "qemu-system-x86_64 is required" >&2; exit 1; }
for bdf in "${GPUS[@]}"; do
  drv=$(basename "$(readlink -f "/sys/bus/pci/devices/$bdf/driver" 2>/dev/null)" 2>/dev/null || true)
  if [[ "$drv" != "vfio-pci" ]]; then
    echo "$bdf is on driver '${drv:-none}', not vfio-pci. Bind it first (see the runbook)." >&2
    exit 1
  fi
done

TOKEN=${DCL_STREAM_TOKEN:-$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')}
[[ ${#TOKEN} -eq 64 ]] || { echo "DCL_STREAM_TOKEN must be 64 hex characters" >&2; exit 1; }

# Changes stay out of the image unless asked (games you install are lost on exit).
SNAPSHOT=on
[[ "${DCL_KEEP_CHANGES:-0}" == "1" ]] && SNAPSHOT=off

ip=$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for (i=1;i<=NF;i++) if ($i=="src") print $(i+1)}')
cat <<EOF

  Guest starting. When the desktop is up (1-2 minutes), on the PC you play from:

    dcl-stream-client --addr ${ip:-<this-pc-ip>}:${PORT} --token ${TOKEN}

  The token is this session's password: do not share it.
  Guest console log: guest-serial.log    Stop: Ctrl+C here.

EOF

dev_args=()
for bdf in "${GPUS[@]}"; do
  dev_args+=(-device "vfio-pci,host=$bdf")
done

exec qemu-system-x86_64 \
  -enable-kvm -machine q35 -cpu host -smp "cores=$CPUS" -m "$MEM" \
  -drive "file=$IMAGE,if=virtio,format=raw,snapshot=$SNAPSHOT" \
  "${dev_args[@]}" \
  -netdev "user,id=n0,hostfwd=udp::${PORT}-:47100" -device virtio-net-pci,netdev=n0 \
  -smbios "type=11,value=dcl.stream_token=$TOKEN" \
  -display none -serial file:guest-serial.log
