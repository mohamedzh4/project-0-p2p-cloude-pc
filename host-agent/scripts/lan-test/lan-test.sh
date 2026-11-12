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
# Writes lan-test-report-<time>.txt next to this script (no password in it).
set -euo pipefail
umask 077

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${DCL_LAN_TEST_ENV:-"$HERE/lan-test.env"}"
STAMP="$(date +%Y%m%d-%H%M%S)"
REPORT="$HERE/lan-test-report-$STAMP.txt"
CHECK_ONLY=0
[[ "${1:-}" == "--check" ]] && CHECK_ONLY=1

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
    LAN_TEST_VCPUS|LAN_TEST_MEMORY_GIB) printf -v "$key" '%s' "$value" ;;
    *) fail "Unsupported key in lan-test.env: $key" ;;
  esac
done <"$ENV_FILE"

: "${IMAGE_STANDARD_DIGEST:?IMAGE_STANDARD_DIGEST is required in lan-test.env}"
[[ "$IMAGE_STANDARD_DIGEST" =~ ^[0-9a-fA-F]{64}$ ]] ||
  fail "IMAGE_STANDARD_DIGEST must be 64 hexadecimal characters."
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

IMAGE_DIR="${XDG_CACHE_HOME:-"$HOME/.cache"}/dcl-host-agent/images/standard"
mkdir -p "$IMAGE_DIR"
free_gib=$(( $(df -Pk "$IMAGE_DIR" | awk 'NR==2 {print $4}') / 1024 / 1024 ))
log "Free disk in $IMAGE_DIR: ${free_gib} GiB"
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

# --- 4. Guest image (same cache path as linux-host-setup.sh) ---------------
log ""
log "== Preparing the guest image =="
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
    curl --fail --location --retry 5 -C - -o "$RAW_IMAGE.part" "$IMAGE_RAW_URL"
    mv "$RAW_IMAGE.part" "$RAW_IMAGE"
  elif [[ -n "${IMAGE_QCOW2_URL:-}" ]]; then
    log "Downloading guest image (several GB)..."
    curl --fail --location --retry 5 -C - -o "$IMAGE_DIR/rootfs.qcow2.part" "$IMAGE_QCOW2_URL"
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
printf '%s' "$PASSWORD" >"$PW_FILE"

if command -v ufw >/dev/null && sudo ufw status 2>/dev/null | grep -q '^Status: active'; then
  subnet="$(ip -4 route | awk -v ip="$LAN_IP" '$0 ~ "src " ip && $1 ~ "/" {print $1; exit}')"
  UFW_RULE="from ${subnet:-any} to any port $PORT proto tcp"
  # shellcheck disable=SC2086
  sudo ufw allow $UFW_RULE >/dev/null
  log "Opened firewall port $PORT for ${subnet:-any}."
fi

# Same flags as host-agent qemu_kvm.rs, plus:
#  -snapshot  writes go to a temp file; the signed image stays unchanged
#  hostfwd    publishes guest KasmVNC 8443 on this PC's LAN port
#  fw_cfg     LAN password -> guest turns on KasmVNC basic auth
# shellcheck disable=SC2054  # commas are inside QEMU option values
qemu_args=(
  -name dcl-lan-test
  -enable-kvm
  -machine q35
  -cpu host
  -smp "cores=$VCPUS"
  -m "${MEM_GIB}G"
  -drive "file=$RAW_IMAGE,if=virtio,format=raw"
  -snapshot
  -display none
  -monitor none
  -serial "file:$SERIAL_LOG"
  -no-reboot
  -netdev "user,id=net0,hostfwd=tcp:0.0.0.0:$PORT-:8443"
  -device virtio-net-pci,netdev=net0
  -fw_cfg "name=opt/org.dcl/lan-password,file=$PW_FILE"
)
boot_start=$SECONDS
qemu-system-x86_64 "${qemu_args[@]}" >"$QEMU_LOG" 2>&1 &
QEMU_PID=$!
log "VM started (pid $QEMU_PID). Waiting for the desktop (up to 5 minutes)..."

ready=0
while (( SECONDS - boot_start < 300 )); do
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
  tail -n 40 "$SERIAL_LOG" 2>/dev/null | tee -a "$REPORT" || true
  fail "The desktop did not answer within 5 minutes."
fi
log "Desktop answered after ${boot_secs}s (HTTP $code)."
if [[ "$code" != "401" ]]; then
  log "WARNING: expected a password prompt (HTTP 401) but got HTTP $code."
  log "WARNING: the guest image may be older than the LAN password change."
fi
RESULT="PASS: desktop reachable after ${boot_secs}s"

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
