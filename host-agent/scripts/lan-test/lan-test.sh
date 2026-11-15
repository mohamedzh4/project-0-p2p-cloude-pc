#!/usr/bin/env bash
# DCL LAN hardware test (Linux, QEMU+KVM).
#
# Boots the Standard guest on this PC and prints a link + password. Anyone
# on the same Wi-Fi/LAN opens the link to reach the XFCE desktop. No
# Matchmaker, gateway, Headscale or renterd: this only proves that the
# hypervisor + guest image + KasmVNC work on real hardware.
#
#   bash lan-test.sh             # run the test, Ctrl+C to stop
#   bash lan-test.sh --check     # only check this PC, do not boot
#   bash lan-test.sh --cleanup   # undo everything the test changed
#   bash lan-test.sh --yes       # run without questions (defaults/env)
#
# Two modes (lan-test.env LAN_TEST_MODE, default picked automatically):
#   quick   no image build needed: official Ubuntu 24.04 cloud image
#           (pinned SHA-256) + seed.iso from the kit; the first boot installs
#           XFCE + KasmVNC with the same provision-kasm.sh as the real image.
#   signed  the Packer-built Standard guest (IMAGE_STANDARD_DIGEST set).
#
# VM size: asked at start (Enter keeps the smallest working default), or set
# LAN_TEST_VCPUS / LAN_TEST_MEMORY_GIB / LAN_TEST_DISK_GIB in lan-test.env.
#
# Writes lan-test-report-<time>.txt next to this script (no password in it).
set -euo pipefail
umask 077

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${DCL_LAN_TEST_ENV:-"$HERE/lan-test.env"}"
STAMP="$(date +%Y%m%d-%H%M%S)"
REPORT="$HERE/lan-test-report-$STAMP.txt"

ACTION=run
ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    --check) ACTION=check ;;
    --cleanup) ACTION=cleanup ;;
    --yes|-y) ASSUME_YES=1 ;;
    *) echo "Usage: bash lan-test.sh [--check | --cleanup] [--yes]" >&2; exit 2 ;;
  esac
done

# Quick mode base image. Dated folder: the bare release/ link moves.
UBUNTU_URL="https://cloud-images.ubuntu.com/releases/noble/release-20261017/ubuntu-24.04-server-cloudimg-amd64.img"
UBUNTU_SHA256="04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94"

# Smallest sizes that run the desktop (2 GiB RAM installed and ran XFCE +
# KasmVNC in testing). Defaults = minimum except CPU.
MIN_VCPUS=1
MIN_MEM_GIB=2
MIN_DISK_GIB=6
DEFAULT_MEM_GIB=2
DEFAULT_DISK_GIB=8

CACHE="${XDG_CACHE_HOME:-"$HOME/.cache"}"
STATE_DIR="$CACHE/dcl-lan-test"
QUICK_DIR="$STATE_DIR/quick"
IMAGE_DIR="$CACHE/dcl-host-agent/images/standard"
# What this test changed, so --cleanup can undo exactly that.
FW_STATE="$STATE_DIR/firewall-rules"
PKG_STATE="$STATE_DIR/installed-packages"
KVM_STATE="$STATE_DIR/added-to-kvm-group"

QEMU_PID=""
SUDO_KEEPALIVE_PID=""
RUN_DIR=""
RESULT="FAIL"

log() { printf '%s\n' "$*" | tee -a "$REPORT"; }
fail() {
  log "FAIL: $*"
  exit 1
}

is_interactive() { [[ -t 0 && -t 1 && "$ASSUME_YES" -eq 0 ]]; }

ask_yes() {
  # ask_yes "Question?" -> 0 for yes. --yes answers yes; no terminal: no.
  local reply
  (( ASSUME_YES )) && return 0
  [[ -t 0 ]] || return 1
  read -r -p "$1 [y/N] " reply
  [[ "$reply" =~ ^[Yy] ]]
}

detect_pkg() {
  if command -v apt-get >/dev/null; then
    echo apt
  elif command -v pacman >/dev/null; then
    echo pacman
  else
    echo none
  fi
}

# --- firewall bookkeeping -------------------------------------------------
remember_fw() { mkdir -p "$STATE_DIR"; printf '%s %s\n' "$1" "$2" >>"$FW_STATE"; }

forget_fw() {
  [[ -f "$FW_STATE" ]] || return 0
  grep -vxF -- "$1 $2" "$FW_STATE" >"$FW_STATE.tmp" || true
  mv "$FW_STATE.tmp" "$FW_STATE"
  [[ -s "$FW_STATE" ]] || rm -f "$FW_STATE"
}

remove_fw_rule() {
  # remove_fw_rule KIND RULE -> 0 if removed
  local kind="$1" rule="$2"
  case "$kind" in
    # shellcheck disable=SC2086
    ufw) sudo -n ufw delete allow $rule >/dev/null 2>&1 || sudo ufw delete allow $rule >/dev/null ;;
    firewalld)
      sudo -n firewall-cmd --quiet --remove-rich-rule="$rule" >/dev/null 2>&1 ||
        sudo firewall-cmd --quiet --remove-rich-rule="$rule" ;;
    *) return 1 ;;
  esac
}

remove_recorded_fw_rules() {
  local kind rule left=0
  [[ -f "$FW_STATE" ]] || return 0
  while read -r kind rule; do
    [[ -n "$kind" ]] || continue
    if remove_fw_rule "$kind" "$rule"; then
      forget_fw "$kind" "$rule"
      log "Closed firewall: $kind $rule"
    else
      left=1
      log "Could not remove firewall rule ($kind): $rule"
    fi
  done <"$FW_STATE"
  if (( left )); then
    log "Run 'bash lan-test.sh --cleanup' to remove the remaining firewall rules."
  fi
}

# --- cleanup mode ----------------------------------------------------------
run_cleanup() {
  REPORT=/dev/null
  echo "== DCL LAN test cleanup =="
  if pkill -f "qemu-system-x86_64 -name dcl-lan-test" 2>/dev/null; then
    echo "Stopped a running test VM."
  fi
  if [[ -f "$FW_STATE" ]]; then
    echo "Removing firewall rules added by the test..."
    remove_recorded_fw_rules
  fi
  if command -v ufw >/dev/null && sudo ufw status 2>/dev/null | grep -q "8443/tcp"; then
    echo "ufw still lists port 8443 rules (maybe from an older version of this test):"
    sudo ufw status numbered | grep "8443/tcp" || true
    echo "Delete them with: sudo ufw delete <number>  (highest number first)"
  fi

  if [[ -s "$PKG_STATE" ]]; then
    local pkgs pkg_mgr
    pkgs="$(sort -u "$PKG_STATE" | tr '\n' ' ')"
    pkg_mgr="$(detect_pkg)"
    echo "Packages installed by the test: $pkgs"
    if ask_yes "Remove them?"; then
      # shellcheck disable=SC2086
      case "$pkg_mgr" in
        apt) sudo apt-get remove --autoremove -y $pkgs ;;
        pacman) sudo pacman -Rns --noconfirm $pkgs ;;
      esac
    else
      echo "Kept. To remove later: $([[ "$pkg_mgr" == pacman ]] && echo "sudo pacman -Rns" || echo "sudo apt-get remove --autoremove") $pkgs"
    fi
  fi

  if [[ -f "$KVM_STATE" ]]; then
    if ask_yes "The test added $USER to the kvm group. Remove $USER from it?"; then
      sudo gpasswd -d "$USER" kvm
    fi
  fi

  if [[ -f "$IMAGE_DIR/rootfs.img" ]]; then
    if ask_yes "Delete the signed test image ($(du -h "$IMAGE_DIR/rootfs.img" | cut -f1)) in $IMAGE_DIR?"; then
      rm -f "$IMAGE_DIR/rootfs.img"
      rmdir -p "$IMAGE_DIR" 2>/dev/null || true
    fi
  fi

  if [[ -d "$STATE_DIR" ]]; then
    echo "Deleting the VM disk and Ubuntu image ($(du -sh "$STATE_DIR" | cut -f1))..."
    rm -rf "$STATE_DIR"
  fi
  rm -rf "${TMPDIR:-/tmp}"/dcl-lan-test.*
  echo "Done. Report files next to this script were kept."
}

if [[ "$ACTION" == cleanup ]]; then
  run_cleanup
  exit 0
fi

# --- run / check -------------------------------------------------------------
cleanup() {
  local code=$?
  if [[ -n "$SUDO_KEEPALIVE_PID" ]]; then
    kill "$SUDO_KEEPALIVE_PID" 2>/dev/null || true
  fi
  if [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
    log "Stopping the virtual machine..."
    kill "$QEMU_PID" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      kill -0 "$QEMU_PID" 2>/dev/null || break
      sleep 1
    done
    kill -9 "$QEMU_PID" 2>/dev/null || true
  fi
  remove_recorded_fw_rules || true
  [[ -n "$RUN_DIR" ]] && rm -rf "$RUN_DIR"
  log "Result: $RESULT"
  log "Finished: $(date "+%Y-%m-%dT%H:%M:%S%z")"
  echo
  echo "Report saved to: $REPORT"
  echo "Send that file back to the project owner."
  echo "To undo everything this test changed: bash lan-test.sh --cleanup"
  exit "$code"
}
trap cleanup EXIT
on_interrupt() {
  [[ "$RESULT" == "FAIL" ]] && RESULT="STOPPED by user before the desktop was ready"
  exit 130
}
trap on_interrupt INT TERM HUP

: >"$REPORT"
log "DCL LAN hardware test (Linux)"
log "Started: $(date "+%Y-%m-%dT%H:%M:%S%z")"

# --- 1. Settings -----------------------------------------------------------
[[ -f "$ENV_FILE" ]] || fail "lan-test.env is missing next to this script."
while IFS='=' read -r key value; do
  key="${key//[[:space:]]/}"
  [[ -z "$key" || "$key" == \#* ]] && continue
  value="${value%$'\r'}"
  value="${value#\"}"
  value="${value%\"}"
  case "$key" in
    IMAGE_STANDARD_DIGEST|IMAGE_RAW_URL|IMAGE_QCOW2_URL|LAN_TEST_PORT|\
    LAN_TEST_VCPUS|LAN_TEST_MEMORY_GIB|LAN_TEST_DISK_GIB|LAN_TEST_MODE|\
    LAN_TEST_SETUP_MINUTES|LAN_TEST_BOOT_MINUTES)
      printf -v "$key" '%s' "$value" ;;
    *) fail "Unsupported key in lan-test.env: $key" ;;
  esac
done <"$ENV_FILE"

MODE="${LAN_TEST_MODE:-}"
if [[ -z "$MODE" ]]; then
  if [[ -n "${IMAGE_STANDARD_DIGEST:-}" ]]; then MODE=signed; else MODE=quick; fi
fi
case "$MODE" in
  signed)
    [[ "${IMAGE_STANDARD_DIGEST:-}" =~ ^[0-9a-fA-F]{64}$ ]] ||
      fail "IMAGE_STANDARD_DIGEST must be 64 hexadecimal characters (signed mode)."
    ;;
  quick)
    [[ -f "$HERE/seed.iso" ]] || fail "seed.iso is missing next to this script (quick mode)."
    ;;
  *) fail "LAN_TEST_MODE must be quick or signed." ;;
esac
SETUP_MIN="${LAN_TEST_SETUP_MINUTES:-45}"
[[ "$SETUP_MIN" =~ ^[0-9]+$ && "$SETUP_MIN" -ge 5 ]] || fail "LAN_TEST_SETUP_MINUTES must be >= 5."
BOOT_MIN="${LAN_TEST_BOOT_MINUTES:-10}"
[[ "$BOOT_MIN" =~ ^[0-9]+$ && "$BOOT_MIN" -ge 2 ]] || fail "LAN_TEST_BOOT_MINUTES must be >= 2."
PORT="${LAN_TEST_PORT:-8443}"
[[ "$PORT" =~ ^[0-9]+$ && "$PORT" -ge 1024 && "$PORT" -le 65535 ]] ||
  fail "LAN_TEST_PORT must be 1024-65535."

# --- 2. Machine checks -----------------------------------------------------
log ""
log "== Checking this PC =="
[[ "$(uname -s)" == "Linux" ]] || fail "This script is for Linux. Use lan-test.cmd on Windows."
[[ "$(uname -m)" == "x86_64" ]] || fail "Only x86-64 PCs are supported (found $(uname -m))."
[[ "${EUID}" -ne 0 ]] || fail "Run as your normal user (not root); sudo is used when needed."
PKG="$(detect_pkg)"
[[ "$PKG" != none ]] || fail "Only Ubuntu/Debian (apt) and Arch (pacman) are supported."

if [[ -r /etc/os-release ]]; then
  # shellcheck disable=SC1091
  log "OS: $(. /etc/os-release && echo "$PRETTY_NAME")"
fi
log "Kernel: $(uname -r)"
log "Package manager: $PKG"
# Test hook only (Rosetta in Docker fakes /proc/cpuinfo); normally unset.
CPUINFO="${DCL_LAN_TEST_CPUINFO:-/proc/cpuinfo}"
log "CPU: $(grep -m1 'model name' "$CPUINFO" | cut -d: -f2- | sed 's/^ *//')"
log "CPU threads: $(nproc)"

if ! grep -Eq '(vmx|svm)' "$CPUINFO"; then
  fail "CPU virtualization (VT-x / AMD-V) is off. Enable it in BIOS/UEFI, then run again."
fi
log "CPU virtualization flag: present"

if [[ ! -e /dev/kvm ]]; then
  sudo modprobe kvm_intel 2>/dev/null || sudo modprobe kvm_amd 2>/dev/null || true
fi
[[ -e /dev/kvm ]] || fail "/dev/kvm is missing. Enable virtualization in BIOS/UEFI."

if [[ ! -r /dev/kvm || ! -w /dev/kvm ]]; then
  if [[ "${DCL_LAN_TEST_SG:-}" == "1" ]]; then
    fail "/dev/kvm is still not usable after joining the kvm group. Log out and back in, then run again."
  fi
  log "Adding $USER to the kvm group..."
  sudo usermod -aG kvm "$USER"
  mkdir -p "$STATE_DIR" && touch "$KVM_STATE"
  trap - EXIT
  export DCL_LAN_TEST_SG=1
  exec sg kvm -c "bash $(printf '%q' "$0") $(printf '%q ' "$@")"
fi
log "/dev/kvm: usable"

# --- 3. VM size --------------------------------------------------------------
mkdir -p "$STATE_DIR" "$QUICK_DIR" "$IMAGE_DIR"
host_threads="$(nproc)"
mem_avail_kib="$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)"
mem_avail_gib=$(( mem_avail_kib / 1024 / 1024 ))
free_gib=$(( $(df -Pk "$CACHE" | awk 'NR==2 {print $4}') / 1024 / 1024 ))
log "Free RAM: ${mem_avail_gib} GiB"
log "Free disk in $CACHE: ${free_gib} GiB"

OVERLAY="$QUICK_DIR/overlay.qcow2"
BASE="$QUICK_DIR/$(basename "$UBUNTU_URL" .img)-${UBUNTU_SHA256:0:12}.img"
# Leave one thread and 1 GiB RAM / disk to the host.
max_vcpus=$(( host_threads - 1 ))
max_mem=$(( mem_avail_gib - 1 ))
max_disk=$(( free_gib - 1 ))
[[ -f "$BASE" ]] || max_disk=$(( max_disk - 1 ))
default_vcpus=2
(( max_vcpus < 2 )) && default_vcpus=1

ask_size() {
  # ask_size VAR "label" default min max unit
  local var="$1" label="$2" def="$3" min="$4" max="$5" unit="$6" reply
  if [[ -n "${!var:-}" ]]; then
    return 0
  fi
  if ! is_interactive || [[ "$ACTION" != run ]]; then
    printf -v "$var" '%s' "$def"
    return 0
  fi
  while true; do
    read -r -p "  $label [$min-$max $unit, Enter = $def]: " reply
    reply="${reply:-$def}"
    if [[ "$reply" =~ ^[0-9]+$ ]] && (( reply >= min && reply <= max )); then
      printf -v "$var" '%s' "$reply"
      return 0
    fi
    echo "  Please type a number from $min to $max."
  done
}

(( max_vcpus >= MIN_VCPUS )) || fail "This PC needs at least 2 CPU threads (1 for the VM, 1 for itself)."
(( max_mem >= MIN_MEM_GIB )) ||
  fail "Not enough free RAM: the smallest VM needs ${MIN_MEM_GIB} GiB plus 1 GiB for this PC (free: ${mem_avail_gib} GiB). Close other apps."
if [[ "$MODE" == quick && ! -f "$OVERLAY" ]] && (( max_disk < MIN_DISK_GIB )); then
  fail "Not enough free disk: the smallest VM disk is ${MIN_DISK_GIB} GiB plus room for the Ubuntu image (free: ${free_gib} GiB)."
fi

if is_interactive && [[ "$ACTION" == run ]]; then
  echo
  echo "VM size (press Enter to keep the smallest working size):"
fi
ask_size LAN_TEST_VCPUS "CPU cores for the VM" "$default_vcpus" "$MIN_VCPUS" "$max_vcpus" "cores"
ask_size LAN_TEST_MEMORY_GIB "RAM for the VM" "$DEFAULT_MEM_GIB" "$MIN_MEM_GIB" "$max_mem" "GiB"
if [[ "$MODE" == quick && ! -f "$OVERLAY" ]]; then
  default_disk=$DEFAULT_DISK_GIB
  (( max_disk < default_disk )) && default_disk=$max_disk
  ask_size LAN_TEST_DISK_GIB "VM disk" "$default_disk" "$MIN_DISK_GIB" "$max_disk" "GiB"
fi
VCPUS="$LAN_TEST_VCPUS"
MEM_GIB="$LAN_TEST_MEMORY_GIB"
# 0 = keep the existing VM disk as it is (set LAN_TEST_DISK_GIB to grow it).
DISK_GIB="${LAN_TEST_DISK_GIB:-0}"
[[ "$VCPUS" =~ ^[0-9]+$ ]] && (( VCPUS >= MIN_VCPUS && VCPUS <= max_vcpus )) ||
  fail "LAN_TEST_VCPUS must be ${MIN_VCPUS}-${max_vcpus} on this PC."
[[ "$MEM_GIB" =~ ^[0-9]+$ ]] && (( MEM_GIB >= MIN_MEM_GIB && MEM_GIB <= max_mem )) ||
  fail "LAN_TEST_MEMORY_GIB must be ${MIN_MEM_GIB}-${max_mem} on this PC (free RAM ${mem_avail_gib} GiB)."
[[ "$DISK_GIB" =~ ^[0-9]+$ ]] && (( DISK_GIB == 0 || DISK_GIB >= MIN_DISK_GIB )) ||
  fail "LAN_TEST_DISK_GIB must be at least ${MIN_DISK_GIB}."
if [[ "$MODE" == quick && ! -f "$OVERLAY" ]] && (( DISK_GIB > max_disk )); then
  fail "LAN_TEST_DISK_GIB=${DISK_GIB} does not fit: ${free_gib} GiB free here."
fi
if [[ "$MODE" == quick && "$DISK_GIB" -gt 0 ]]; then
  log "VM size: ${VCPUS} CPU, ${MEM_GIB} GiB RAM, ${DISK_GIB} GiB disk"
elif [[ "$MODE" == quick ]]; then
  log "VM size: ${VCPUS} CPU, ${MEM_GIB} GiB RAM, existing VM disk"
else
  log "VM size: ${VCPUS} CPU, ${MEM_GIB} GiB RAM"
fi

# --- 4. Network --------------------------------------------------------------
# Local networks on real interfaces, as "dev network address" lines. VPNs
# (wg0, tun0, tailscale0), Docker and VM bridges are skipped: with a VPN up,
# the default route goes through the VPN, and its address is useless on the
# LAN (seen on an Arch PC with WireGuard, 2026-11-14).
lan_networks() {
  ip -4 route show proto kernel scope link 2>/dev/null |
    awk '{dev = ""; src = ""
          for (i = 1; i <= NF; i++) { if ($i == "dev") dev = $(i + 1); if ($i == "src") src = $(i + 1) }
          if (dev != "" && src != "") print dev, $1, src}' |
    grep -Ev '^(lo|wg|tun|tap|tailscale|zt|docker|br-|virbr|veth|vmnet|vboxnet|lxc|lxd|podman|cni|flannel|cali)' || true
}
LAN_NETS=()
while IFS= read -r line; do LAN_NETS+=("$line"); done < <(lan_networks)
(( ${#LAN_NETS[@]} > 0 )) || fail "No Wi-Fi/Ethernet network found. Connect to the Wi-Fi/LAN first."
# Main address: the interface of the default route if it is a LAN one,
# otherwise the first LAN interface.
default_dev="$(ip -4 route get 1.1.1.1 2>/dev/null |
  awk '{for (i = 1; i <= NF; i++) if ($i == "dev") {print $(i + 1); exit}}')"
LAN_IP=""
LAN_IPS=()
LAN_SUBNETS=()
for entry in "${LAN_NETS[@]}"; do
  read -r dev net src <<<"$entry"
  LAN_SUBNETS+=("$net")
  if [[ "$dev" == "$default_dev" && -z "$LAN_IP" ]]; then
    LAN_IP="$src"
  else
    LAN_IPS+=("$src")
  fi
  log "LAN network: $dev $net (address $src)"
done
if [[ -z "$LAN_IP" ]]; then
  LAN_IP="${LAN_IPS[0]}"
  LAN_IPS=("${LAN_IPS[@]:1}")
  log "Note: the default route is not a Wi-Fi/Ethernet interface (VPN?): ${default_dev:-none}."
fi
log "LAN address: $LAN_IP"

if ss -Hltn "sport = :$PORT" 2>/dev/null | grep -q .; then
  fail "Port $PORT is already in use. Set LAN_TEST_PORT=8444 in lan-test.env."
fi

if [[ "$ACTION" == check ]]; then
  RESULT="CHECK PASSED (no VM started)"
  log "This PC passes the checks."
  exit 0
fi

# Ask for the sudo password once and keep it valid for the whole test, so
# the firewall rules can still be removed when the test stops (an expired
# sudo left two ufw rules behind on the Arch PC, 2026-11-14).
sudo -v || fail "sudo is needed to install QEMU and open the firewall."
( while kill -0 "$$" 2>/dev/null; do sudo -n -v 2>/dev/null || true; sleep 50; done ) &
SUDO_KEEPALIVE_PID=$!

# --- 5. Tools --------------------------------------------------------------
log ""
log "== Installing QEMU if needed =="
if ! command -v qemu-system-x86_64 >/dev/null || ! command -v qemu-img >/dev/null ||
   ! command -v curl >/dev/null; then
  if [[ "$PKG" == apt ]]; then
    want_pkgs=(qemu-system-x86 qemu-utils curl)
    for p in "${want_pkgs[@]}"; do
      dpkg -s "$p" >/dev/null 2>&1 || echo "$p" >>"$PKG_STATE"
    done
    sudo apt-get update
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y "${want_pkgs[@]}"
  else
    want_pkgs=(qemu-base curl)
    for p in "${want_pkgs[@]}"; do
      pacman -Q "$p" >/dev/null 2>&1 || echo "$p" >>"$PKG_STATE"
    done
    # No -Sy: a refresh without a full upgrade is a partial upgrade on Arch.
    sudo pacman -S --needed --noconfirm "${want_pkgs[@]}" ||
      fail "pacman could not install qemu-base. Run 'sudo pacman -Syu', then run this again."
  fi
fi
log "QEMU: $(qemu-system-x86_64 --version | head -n1)"

# --- 6. Guest image ----------------------------------------------------------
log ""
log "== Preparing the guest image =="
FIRST_SETUP=0
if [[ "$MODE" == signed ]]; then
  # Same cache path as linux-host-setup.sh.
  RAW_IMAGE="$IMAGE_DIR/rootfs.img"
  want="${IMAGE_STANDARD_DIGEST,,}"

  verify_raw() {
    log "Checking image SHA-256 (takes a minute)..."
    local got
    got="$(sha256sum "$RAW_IMAGE" | awk '{print $1}')"
    [[ "$got" == "$want" ]]
  }

  if [[ -f "$RAW_IMAGE" ]] && ! verify_raw; then
    log "Cached image does not match; preparing it again."
    rm -f "$RAW_IMAGE"
  fi
  if [[ ! -f "$RAW_IMAGE" ]]; then
    if [[ -f "$HERE/rootfs.img" ]]; then
      cp "$HERE/rootfs.img" "$RAW_IMAGE"
    elif [[ -f "$HERE/rootfs.qcow2" ]]; then
      log "Converting rootfs.qcow2 to raw..."
      qemu-img convert -p -O raw "$HERE/rootfs.qcow2" "$RAW_IMAGE"
    elif [[ -n "${IMAGE_RAW_URL:-}" ]]; then
      log "Downloading guest image (several GB)..."
      curl --fail --location --retry 5 --retry-all-errors -C - -o "$RAW_IMAGE.part" "$IMAGE_RAW_URL"
      mv "$RAW_IMAGE.part" "$RAW_IMAGE"
    elif [[ -n "${IMAGE_QCOW2_URL:-}" ]]; then
      log "Downloading guest image (several GB)..."
      curl --fail --location --retry 5 --retry-all-errors -C - -o "$IMAGE_DIR/rootfs.qcow2.part" "$IMAGE_QCOW2_URL"
      mv "$IMAGE_DIR/rootfs.qcow2.part" "$IMAGE_DIR/rootfs.qcow2"
      qemu-img convert -p -O raw "$IMAGE_DIR/rootfs.qcow2" "$RAW_IMAGE"
      rm -f "$IMAGE_DIR/rootfs.qcow2"
    else
      fail "No guest image. Put rootfs.qcow2 next to this script, or set IMAGE_QCOW2_URL."
    fi
    if ! verify_raw; then
      rm -f "$RAW_IMAGE"
      fail "Guest image SHA-256 does not match lan-test.env. Deleted it; not booting."
    fi
  fi
  log "Guest image verified."
else
  SEED="$QUICK_DIR/seed.iso"
  DONE_MARK="$QUICK_DIR/setup-done"
  if [[ ! -f "$BASE" ]]; then
    log "Downloading Ubuntu 24.04 cloud image (about 600 MB)..."
    curl --fail --location --retry 5 --retry-all-errors -C - -o "$BASE.part" "$UBUNTU_URL" ||
      fail "Ubuntu image download failed. Check the internet connection."
    got="$(sha256sum "$BASE.part" | awk '{print $1}')"
    if [[ "$got" != "$UBUNTU_SHA256" ]]; then
      rm -f "$BASE.part"
      fail "Ubuntu image SHA-256 does not match the pinned value. Deleted it; not booting."
    fi
    mv "$BASE.part" "$BASE"
  fi
  log "Ubuntu image verified ($UBUNTU_SHA256)."
  seed_sum="$(sha256sum "$HERE/seed.iso" | awk '{print $1}')"
  if [[ -f "$OVERLAY" ]] && { [[ ! -f "$DONE_MARK" ]] || [[ "$(cat "$DONE_MARK")" != "$seed_sum" ]]; }; then
    log "Previous setup was unfinished or the kit changed; starting the desktop setup again."
    rm -f "$OVERLAY" "$DONE_MARK"
  fi
  cp "$HERE/seed.iso" "$SEED"
  if [[ ! -f "$OVERLAY" ]]; then
    # DISK_GIB is 0 when an old disk was found earlier and has just been reset.
    (( DISK_GIB > 0 )) || DISK_GIB=$DEFAULT_DISK_GIB
    qemu-img create -q -f qcow2 -F qcow2 -b "$BASE" "$OVERLAY" "${DISK_GIB}G"
    FIRST_SETUP=1
  else
    # Grow (never shrink) an existing disk; the guest grows its partition
    # on the next boot.
    cur_bytes="$(qemu-img info --output=json "$OVERLAY" |
      sed -n 's/.*"virtual-size": *\([0-9]*\).*/\1/p' | head -n1)"
    want_bytes=$(( DISK_GIB * 1024 * 1024 * 1024 ))
    if [[ -n "$cur_bytes" ]] && (( DISK_GIB > 0 && want_bytes > cur_bytes )); then
      qemu-img resize -q "$OVERLAY" "${DISK_GIB}G"
      log "VM disk grown to ${DISK_GIB} GiB."
    fi
  fi
  log "VM disk uses $(du -h "$OVERLAY" | cut -f1) on this PC (+ $(du -h "$BASE" | cut -f1) Ubuntu image)."
fi

# --- 7. Boot ---------------------------------------------------------------
log ""
log "== Starting the virtual machine =="
PASSWORD="$(head -c 512 /dev/urandom | LC_ALL=C tr -dc 'a-hjkmnp-z2-9')"
PASSWORD="${PASSWORD:0:12}"
[[ "${#PASSWORD}" -eq 12 ]] || fail "Could not generate a password."

RUN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/dcl-lan-test.XXXXXX")"
SERIAL_LOG="$HERE/lan-test-serial-$STAMP.log"
QEMU_LOG="$RUN_DIR/qemu.log"
PW_FILE="$RUN_DIR/lan-password"
# SMBIOS OEM string, not fw_cfg: Ubuntu's cloud kernel has no qemu_fw_cfg.
printf 'dcl-lan-password=%s' "$PASSWORD" >"$PW_FILE"

# Open the port for every LAN network found above (local networks only).
# Each rule is recorded first, so --cleanup can remove it even if this
# window is closed.
if command -v ufw >/dev/null && sudo ufw status 2>/dev/null | grep -q '^Status: active'; then
  for net in "${LAN_SUBNETS[@]}"; do
    rule="from $net to any port $PORT proto tcp"
    remember_fw ufw "$rule"
    # shellcheck disable=SC2086
    sudo ufw allow $rule >/dev/null || fail "ufw refused the rule for port $PORT ($net)."
    log "Opened firewall port $PORT for $net (ufw, until the test stops)."
  done
elif command -v firewall-cmd >/dev/null && sudo firewall-cmd --state >/dev/null 2>&1; then
  for net in "${LAN_SUBNETS[@]}"; do
    # Runtime-only rule (not saved across reboots); removed again on exit.
    rule="rule family=ipv4 source address=$net port port=$PORT protocol=tcp accept"
    remember_fw firewalld "$rule"
    sudo firewall-cmd --quiet --add-rich-rule="$rule" ||
      fail "firewalld refused the rule for port $PORT ($net)."
    log "Opened firewall port $PORT for $net (firewalld, until the test stops)."
  done
else
  log "No ufw/firewalld running. If another firewall is on, allow TCP $PORT from the LAN."
fi

# Same flags as host-agent qemu_kvm.rs, plus:
#  -snapshot  (signed) writes go to a temp file; the signed image is unchanged
#  overlay    (quick) desktop install persists so later runs boot fast
#  hostfwd    publishes guest KasmVNC 8443 on this PC's LAN port
#  smbios 11  LAN password (from a file, not argv) -> KasmVNC basic auth on
if [[ "$MODE" == signed ]]; then
  disk_args=(-drive "file=$RAW_IMAGE,if=virtio,format=raw" -snapshot)
else
  # shellcheck disable=SC2054
  disk_args=(-drive "file=$OVERLAY,if=virtio,format=qcow2"
    -drive "file=$SEED,if=virtio,format=raw,readonly=on")
fi
# shellcheck disable=SC2054  # commas are inside QEMU option values
qemu_args=(
  -name dcl-lan-test
  -enable-kvm
  -machine q35
  -cpu host
  -smp "cores=$VCPUS"
  -m "${MEM_GIB}G"
  "${disk_args[@]}"
  -display none
  -monitor none
  -serial "file:$SERIAL_LOG"
  -no-reboot
  -netdev "user,id=net0,hostfwd=tcp:0.0.0.0:$PORT-:8443"
  -device virtio-net-pci,netdev=net0
  -smbios "type=11,path=$PW_FILE"
)
boot_start=$SECONDS
qemu-system-x86_64 "${qemu_args[@]}" >"$QEMU_LOG" 2>&1 &
QEMU_PID=$!
if (( FIRST_SETUP )); then
  limit=$(( SETUP_MIN * 60 ))
  log "VM started (pid $QEMU_PID). First run: installing the desktop inside the VM."
  log "This downloads ~500 MB and takes 10-30 minutes (limit ${SETUP_MIN} min)."
else
  limit=$(( BOOT_MIN * 60 ))
  log "VM started (pid $QEMU_PID). Waiting for the desktop (up to ${BOOT_MIN} minutes)..."
fi

ready=0
next_note=60
while (( SECONDS - boot_start < limit )); do
  if (( SECONDS - boot_start >= next_note )); then
    echo "  ... still starting ($(( (SECONDS - boot_start) / 60 )) min)"
    next_note=$(( next_note + 60 ))
  fi
  if grep -q "DCL-QUICK-SETUP-FAILED" "$SERIAL_LOG" 2>/dev/null; then
    log "Desktop setup inside the VM failed. Guest console:"
    tail -n 60 "$SERIAL_LOG" | tee -a "$REPORT"
    rm -f "${OVERLAY:-}"
    fail "Desktop setup failed inside the VM (see above)."
  fi
  if ! kill -0 "$QEMU_PID" 2>/dev/null; then
    log "QEMU stopped early. Its output:"
    tee -a "$REPORT" <"$QEMU_LOG"
    fail "The virtual machine did not start."
  fi
  code="$(curl -sk -o /dev/null -m 3 -w '%{http_code}' "https://127.0.0.1:$PORT/" || true)"
  if [[ "$code" =~ ^[1-5][0-9][0-9]$ ]]; then
    ready=1
    break
  fi
  sleep 3
done
rm -f "$PW_FILE"
boot_secs=$(( SECONDS - boot_start ))
if (( ! ready )); then
  log "Last lines of the guest console:"
  tail -n 80 "$SERIAL_LOG" 2>/dev/null | tee -a "$REPORT" || true
  if [[ "$MODE" == quick && "$FIRST_SETUP" -eq 0 ]]; then
    # Known open issue: the installed desktop may not come back on a later
    # boot. Drop the marker so the next run reinstalls from scratch.
    rm -f "$DONE_MARK"
    log "Run lan-test again: it will reinstall the desktop (first-run time)."
  fi
  fail "The desktop did not answer within $(( limit / 60 )) minutes."
fi
log "Desktop answered after ${boot_secs}s (HTTP $code)."
if [[ "$code" != "401" ]]; then
  # Fail closed: the port is open to the LAN and the desktop has no password.
  log "The desktop answered HTTP $code instead of asking for a password (401)."
  log "The guest image is older than the LAN password change, or the password did not reach the VM."
  fail "Stopped the VM so the desktop is not open to the network without a password."
fi
if [[ "$MODE" == quick && ! -f "$DONE_MARK" ]]; then
  printf '%s' "$seed_sum" >"$DONE_MARK"
fi
RESULT="PASS ($MODE mode): desktop reachable after ${boot_secs}s"

cat <<EOF

============================================================
  The test computer is ready.

  On another device on the SAME Wi-Fi, open:

      https://$LAN_IP:$PORT
$(for ip in "${LAN_IPS[@]}"; do printf '\n  or, from a device on the other network:  https://%s:%s\n' "$ip" "$PORT"; done)
  The browser warns about the certificate: choose
  "Advanced" -> "Continue" (it is a self-signed test cert).

  Username:  ubuntu
  Password:  $PASSWORD

$( [[ "$MODE" == quick ]] && printf '  Speed test: in the VM terminal, type  dcl-bench\n' )

  Keep this window open. Press Ctrl+C to stop the test.
============================================================

EOF
log "Link shown: https://$LAN_IP:$PORT (password not saved)"

wait "$QEMU_PID" || true
QEMU_PID=""
log "The virtual machine stopped by itself."
