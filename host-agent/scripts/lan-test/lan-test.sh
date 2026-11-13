#!/usr/bin/env bash
# DCL LAN hardware test (Linux, QEMU+KVM).
#
# Boots the signed Standard guest on this PC and prints a link + password.
# Anyone on the same Wi-Fi/LAN opens the link to reach the XFCE desktop.
# No Matchmaker, gateway, Headscale or renterd: this only proves that the
# hypervisor + guest image + KasmVNC work on real hardware.
#
#   bash lan-test.sh            # run the test, Ctrl+C to stop
#   bash lan-test.sh --check    # only check this PC, do not boot
#
# Two modes (lan-test.env LAN_TEST_MODE, default picked automatically):
#   quick   no image build needed: official Ubuntu 24.04 cloud image
#           (pinned SHA-256) + seed.iso from the kit; the first boot installs
#           XFCE + KasmVNC with the same provision-kasm.sh as the real image.
#   signed  the Packer-built Standard guest (IMAGE_STANDARD_DIGEST set).
#
# Writes lan-test-report-<time>.txt next to this script (no password in it).
set -euo pipefail
umask 077

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${DCL_LAN_TEST_ENV:-"$HERE/lan-test.env"}"
STAMP="$(date +%Y%m%d-%H%M%S)"
REPORT="$HERE/lan-test-report-$STAMP.txt"
CHECK_ONLY=0
[[ "${1:-}" == "--check" ]] && CHECK_ONLY=1

# Quick mode base image. Dated folder: the bare release/ link moves.
UBUNTU_URL="https://cloud-images.ubuntu.com/releases/noble/release-20261017/ubuntu-24.04-server-cloudimg-amd64.img"
UBUNTU_SHA256="04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94"

QEMU_PID=""
UFW_RULE=""
RESULT="FAIL"

log() { printf '%s\n' "$*" | tee -a "$REPORT"; }
fail() {
  log "FAIL: $*"
  exit 1
}

cleanup() {
  local code=$?
  if [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
    log "Stopping the virtual machine..."
    kill "$QEMU_PID" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      kill -0 "$QEMU_PID" 2>/dev/null || break
      sleep 1
    done
    kill -9 "$QEMU_PID" 2>/dev/null || true
  fi
  if [[ -n "$UFW_RULE" ]]; then
    # shellcheck disable=SC2086
    sudo ufw delete allow $UFW_RULE >/dev/null 2>&1 || true
    log "Closed firewall port."
  fi
  log "Result: $RESULT"
  log "Finished: $(date "+%Y-%m-%dT%H:%M:%S%z")"
  echo
  echo "Report saved to: $REPORT"
  echo "Send that file back to the project owner."
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
    LAN_TEST_VCPUS|LAN_TEST_MEMORY_GIB|LAN_TEST_MODE|LAN_TEST_SETUP_MINUTES|\
    LAN_TEST_BOOT_MINUTES)
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
VCPUS="${LAN_TEST_VCPUS:-2}"
MEM_GIB="${LAN_TEST_MEMORY_GIB:-4}"
[[ "$PORT" =~ ^[0-9]+$ && "$PORT" -ge 1024 && "$PORT" -le 65535 ]] ||
  fail "LAN_TEST_PORT must be 1024-65535."
[[ "$VCPUS" =~ ^[0-9]+$ && "$VCPUS" -ge 2 ]] || fail "LAN_TEST_VCPUS must be >= 2."
[[ "$MEM_GIB" =~ ^[0-9]+$ && "$MEM_GIB" -ge 4 ]] || fail "LAN_TEST_MEMORY_GIB must be >= 4."

# --- 2. Machine checks -----------------------------------------------------
log ""
log "== Checking this PC =="
[[ "$(uname -s)" == "Linux" ]] || fail "This script is for Linux. Use lan-test.cmd on Windows."
[[ "$(uname -m)" == "x86_64" ]] || fail "Only x86-64 PCs are supported (found $(uname -m))."
[[ "${EUID}" -ne 0 ]] || fail "Run as your normal user (not root); sudo is used when needed."
command -v apt-get >/dev/null || fail "Only Ubuntu/Debian are supported (apt-get not found)."

if [[ -r /etc/os-release ]]; then
  # shellcheck disable=SC1091
  log "OS: $(. /etc/os-release && echo "$PRETTY_NAME")"
fi
log "Kernel: $(uname -r)"
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
  trap - EXIT
  export DCL_LAN_TEST_SG=1
  exec sg kvm -c "bash $(printf '%q' "$0") $(printf '%q ' "$@")"
fi
log "/dev/kvm: usable"

total_threads="$(nproc)"
(( total_threads >= VCPUS + 1 )) ||
  fail "Need at least $((VCPUS + 1)) CPU threads (have $total_threads)."
mem_avail_kib="$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)"
mem_avail_gib=$(( mem_avail_kib / 1024 / 1024 ))
log "Free RAM: ${mem_avail_gib} GiB (VM uses ${MEM_GIB} GiB)"
(( mem_avail_kib >= (MEM_GIB * 1024 + 512) * 1024 )) ||
  fail "Not enough free RAM: the test needs about $((MEM_GIB + 1)) GiB free. Close other apps and run again."

log "Mode: $MODE"
CACHE="${XDG_CACHE_HOME:-"$HOME/.cache"}"
IMAGE_DIR="$CACHE/dcl-host-agent/images/standard"
QUICK_DIR="$CACHE/dcl-lan-test/quick"
mkdir -p "$IMAGE_DIR" "$QUICK_DIR"
free_gib=$(( $(df -Pk "$CACHE" | awk 'NR==2 {print $4}') / 1024 / 1024 ))
log "Free disk in $CACHE: ${free_gib} GiB"
(( free_gib >= 20 )) || fail "Need at least 20 GiB free disk for the guest image."

LAN_IP="$(ip -4 route get 1.1.1.1 2>/dev/null |
  awk '{for (i = 1; i <= NF; i++) if ($i == "src") {print $(i + 1); exit}}')"
[[ -n "$LAN_IP" ]] || fail "No network connection found. Connect to the Wi-Fi/LAN first."
log "LAN address: $LAN_IP"

if ss -Hltn "sport = :$PORT" 2>/dev/null | grep -q .; then
  fail "Port $PORT is already in use. Set LAN_TEST_PORT=8444 in lan-test.env."
fi

if (( CHECK_ONLY )); then
  RESULT="CHECK PASSED (no VM started)"
  log "This PC passes the checks."
  exit 0
fi

# --- 3. Tools --------------------------------------------------------------
log ""
log "== Installing QEMU if needed =="
if ! command -v qemu-system-x86_64 >/dev/null || ! command -v qemu-img >/dev/null; then
  sudo apt-get update
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y qemu-system-x86 qemu-utils curl
fi
command -v curl >/dev/null || sudo apt-get install -y curl
log "QEMU: $(qemu-system-x86_64 --version | head -n1)"

# --- 4. Guest image ----------------------------------------------------------
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
  BASE="$QUICK_DIR/$(basename "$UBUNTU_URL" .img)-${UBUNTU_SHA256:0:12}.img"
  OVERLAY="$QUICK_DIR/overlay.qcow2"
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
    qemu-img create -q -f qcow2 -F qcow2 -b "$BASE" "$OVERLAY" 20G
    FIRST_SETUP=1
  fi
fi

# --- 5. Boot ---------------------------------------------------------------
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

if command -v ufw >/dev/null && sudo ufw status 2>/dev/null | grep -q '^Status: active'; then
  subnet="$(ip -4 route | awk -v ip="$LAN_IP" '$0 ~ "src " ip && $1 ~ "/" {print $1; exit}')"
  UFW_RULE="from ${subnet:-any} to any port $PORT proto tcp"
  # shellcheck disable=SC2086
  sudo ufw allow $UFW_RULE >/dev/null
  log "Opened firewall port $PORT for ${subnet:-any}."
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

  The browser warns about the certificate: choose
  "Advanced" -> "Continue" (it is a self-signed test cert).

  Username:  ubuntu
  Password:  $PASSWORD

  Keep this window open. Press Ctrl+C to stop the test.
============================================================

EOF
log "Link shown: https://$LAN_IP:$PORT (password not saved)"

wait "$QEMU_PID" || true
QEMU_PID=""
log "The virtual machine stopped by itself."
