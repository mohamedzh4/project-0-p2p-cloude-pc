#!/usr/bin/env bash
# Pro GPU readiness check for a Linux host. Read-only: changes nothing,
# needs no root. Run it, then send back the report file it writes.
#
#   bash pro-gpu-check.sh
#
# It answers: can this PC hand one GPU to a guest (VFIO) and still keep a
# screen for its owner? See docs/runbooks/gate-pro.md.

set -u
REPORT="${PRO_CHECK_REPORT:-pro-gpu-check-$(hostname)-$(date +%Y%m%d-%H%M%S).txt}"
SYS="${PRO_CHECK_SYSROOT:-}"   # tests point this at a fake tree

exec > >(tee "$REPORT") 2>&1

PASS=0; WARN=0; FAIL=0
ok()   { echo "  [ok]   $*"; PASS=$((PASS + 1)); }
warn() { echo "  [warn] $*"; WARN=$((WARN + 1)); }
bad()  { echo "  [FAIL] $*"; FAIL=$((FAIL + 1)); }
section() { echo; echo "== $*"; }

vendor_name() {
  case "$1" in
    0x10de) echo NVIDIA ;;
    0x1002) echo AMD ;;
    0x8086) echo Intel ;;
    *) echo "vendor $1" ;;
  esac
}

section "Machine"
echo "  host:   $(hostname)"
echo "  kernel: $(uname -r)"
[ -r /etc/os-release ] && echo "  distro: $(. /etc/os-release; echo "${PRETTY_NAME:-unknown}")"
CPU_MODEL=$(grep -m1 'model name' "$SYS/proc/cpuinfo" 2>/dev/null | cut -d: -f2- | sed 's/^ *//')
echo "  cpu:    ${CPU_MODEL:-unknown}"

section "CPU virtualization and KVM"
if grep -qE '\b(vmx|svm)\b' "$SYS/proc/cpuinfo" 2>/dev/null; then
  ok "CPU virtualization (VT-x / AMD-V) is on"
else
  bad "No vmx/svm flag: turn on VT-x / SVM in the BIOS"
fi
if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
  ok "/dev/kvm is readable and writable for $(id -un)"
else
  bad "/dev/kvm missing or not writable for $(id -un) (run linux-host-setup.sh)"
fi

section "IOMMU (VT-d / AMD-Vi)"
CMDLINE=$(cat "$SYS/proc/cmdline" 2>/dev/null || true)
echo "  kernel command line: $CMDLINE"
GROUPS_DIR="$SYS/sys/kernel/iommu_groups"
GROUP_COUNT=$(ls "$GROUPS_DIR" 2>/dev/null | wc -l | tr -d ' ')
if [ "${GROUP_COUNT:-0}" -gt 0 ]; then
  ok "IOMMU is active: $GROUP_COUNT groups"
else
  bad "No IOMMU groups. Turn on VT-d (Intel) or IOMMU/AMD-Vi (AMD) in the BIOS; on Intel also add intel_iommu=on to the kernel command line"
fi
case "$CMDLINE" in *iommu=pt*) ok "iommu=pt is set" ;; *) warn "iommu=pt is not set (optional, usually better host performance)" ;; esac

section "vfio-pci"
if [ -d "$SYS/sys/bus/pci/drivers/vfio-pci" ]; then
  ok "vfio-pci driver is loaded"
elif modinfo vfio-pci >/dev/null 2>&1; then
  warn "vfio-pci exists but is not loaded (sudo modprobe vfio-pci, or add it to /etc/modules-load.d/)"
else
  bad "vfio-pci kernel module not found"
fi

section "QEMU"
QEMU_BIN="${HOST_AGENT_QEMU_BIN:-$(command -v qemu-system-x86_64 || true)}"
if [ -n "$QEMU_BIN" ]; then
  ok "QEMU: $QEMU_BIN ($("$QEMU_BIN" --version 2>/dev/null | head -1))"
else
  bad "qemu-system-x86_64 not found (install your distro's qemu package)"
fi
OVMF=$(ls /usr/share/edk2*/x64/OVMF_CODE*.fd /usr/share/OVMF/OVMF_CODE*.fd /usr/share/edk2/ovmf/OVMF_CODE*.fd 2>/dev/null | head -1)
if [ -n "$OVMF" ]; then ok "UEFI firmware found: $OVMF"; else warn "No OVMF (UEFI) firmware found; the agent boots SeaBIOS today, some GPUs need UEFI"; fi

section "GPUs (PCI display devices)"
CANDIDATES=0
DISPLAY_KEEPERS=0
GPU_LIST=""
for dev in "$SYS"/sys/bus/pci/devices/*; do
  [ -r "$dev/class" ] || continue
  class=$(cat "$dev/class")
  case "$class" in 0x03*) ;; *) continue ;; esac
  bdf=$(basename "$dev")
  vendor=$(cat "$dev/vendor"); device=$(cat "$dev/device")
  driver=$( [ -L "$dev/driver" ] && basename "$(readlink "$dev/driver")" || echo none)
  boot_vga=$(cat "$dev/boot_vga" 2>/dev/null || echo 0)
  group=$( [ -L "$dev/iommu_group" ] && basename "$(readlink "$dev/iommu_group")" || echo none)
  name=""
  command -v lspci >/dev/null && name=$(lspci -s "$bdf" 2>/dev/null | cut -d' ' -f2- )
  [ -n "$name" ] || name="$(vendor_name "$vendor") $vendor:$device"
  # Screens plugged into this card right now.
  screens=""
  for conn in "$SYS"/sys/class/drm/card*-*; do
    [ -r "$conn/status" ] || continue
    [ "$(cat "$conn/status")" = connected ] || continue
    card_dev=$(readlink -f "$(dirname "$conn")/$(basename "$conn" | cut -d- -f1)/device" 2>/dev/null)
    [ "$(basename "$card_dev")" = "$bdf" ] && screens="$screens $(basename "$conn" | cut -d- -f2-)"
  done
  echo
  echo "  $bdf  $name"
  echo "    driver: $driver   boot VGA: $boot_vga   IOMMU group: $group"
  echo "    screens connected:${screens:- none}"
  GPU_LIST="$GPU_LIST $bdf"
  [ -n "$screens" ] && DISPLAY_KEEPERS=$((DISPLAY_KEEPERS + 1))
  if [ "$group" != none ]; then
    others=""
    for member in "$SYS/sys/kernel/iommu_groups/$group/devices/"*; do
      m=$(basename "$member"); [ "$m" = "$bdf" ] && continue
      mclass=$(cat "$member/class" 2>/dev/null)
      # Same card's audio/USB-C functions (same bus:slot) travel with it.
      if [ "${m%.*}" = "${bdf%.*}" ]; then continue; fi
      # PCIe bridges in the group are fine.
      case "$mclass" in 0x0604*) continue ;; esac
      others="$others $m"
    done
    if [ -n "$others" ]; then
      echo "    [warn] shares IOMMU group $group with other devices:$others (they would go to the guest too)"
    elif [ "$boot_vga" != 1 ] && [ -z "$screens" ]; then
      echo "    -> good candidate for Pro: own IOMMU group, not driving the host's screen"
      CANDIDATES=$((CANDIDATES + 1))
    else
      echo "    -> clean IOMMU group, but the host uses it for its screen now"
      CANDIDATES=$((CANDIDATES + 1))
    fi
  fi
done
GPU_COUNT=$(echo $GPU_LIST | wc -w | tr -d ' ')
echo
if [ "$GPU_COUNT" -ge 2 ]; then
  ok "$GPU_COUNT display devices: one can go to the guest, the other keeps the host's screen"
elif [ "$GPU_COUNT" -eq 1 ]; then
  bad "Only one display device: giving it to a guest leaves this PC with no screen. Turn on the iGPU in the BIOS if the CPU has one, or add a second card"
else
  bad "No PCI display device found"
fi
[ "$CANDIDATES" -gt 0 ] && ok "$CANDIDATES GPU(s) sit in a clean IOMMU group" || bad "No GPU sits in a clean IOMMU group"

section "NVIDIA driver (the agent reads UUID and VRAM from it)"
if command -v nvidia-smi >/dev/null; then
  nvidia-smi -L 2>&1 | sed 's/^/  /'
else
  echo "  nvidia-smi not found (fine for AMD/Intel; an NVIDIA card is then listed by PCI address)"
fi
echo "  Agent builds from 2026-11-23 list every GPU above; older builds saw NVIDIA only (issue 35)."
for info in "$SYS"/proc/driver/nvidia/gpus/*/information; do
  [ -r "$info" ] && grep -q 'GPU UUID' "$info" && ok "NVIDIA UUID -> PCI mapping readable: $(basename "$(dirname "$info")")"
done

section "Summary"
echo "  ok: $PASS   warnings: $WARN   failures: $FAIL"
echo "  report saved to: $REPORT"
