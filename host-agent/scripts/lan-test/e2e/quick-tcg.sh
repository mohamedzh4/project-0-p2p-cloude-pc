#!/usr/bin/env bash
# shellcheck disable=SC2015,SC2012,SC2002  # test harness: ok()/bad() always return 0
# Real quick-mode run inside an arm64 Docker container (no KVM on a Mac).
# Real: lan-test.sh, Ubuntu cloud image download + SHA-256, seed.iso,
# cloud-init, provision-kasm.sh, KasmVNC, SMBIOS password, hostfwd.
# Shimmed: -enable-kvm -> TCG, -cpu host -> max, -m 4G -> 2G (Docker VM is
# small), uname -m -> x86_64, /dev/kvm + cpuinfo/meminfo checks.
# Expects /e2e/{lan-test.sh,seed.iso}. Slow: TCG installs XFCE in ~30-90 min.
set -uo pipefail
KIT=/home/friend/kit
if [[ "${1:-}" != --driver ]]; then
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq qemu-system-x86 qemu-utils curl iproute2 sudo procps python3 >/dev/null 2>&1 ||
  { echo "apt failed"; exit 1; }
useradd -m friend
echo "friend ALL=(ALL) NOPASSWD:ALL" >/etc/sudoers.d/friend

mkdir -p /opt/shim
cat >/opt/shim/qemu-system-x86_64 <<'EOF'
#!/usr/bin/env bash
args=()
for a in "$@"; do
  case "$a" in
    -enable-kvm) args+=(-accel tcg,thread=multi) ;;
    host) args+=(max) ;;
    4G) args+=(2G) ;;
    *) args+=("$a") ;;
  esac
done
echo "shim argv: ${args[*]}" >&2
exec /usr/bin/qemu-system-x86_64 "${args[@]}"
EOF
cat >/opt/shim/uname <<'EOF'
#!/bin/sh
[ "$1" = "-m" ] && { echo x86_64; exit 0; }
exec /usr/bin/uname "$@"
EOF
chmod 755 /opt/shim/*
touch /dev/kvm && chmod 666 /dev/kvm
printf 'model name : Docker TCG stand-in\nflags : vmx\n' >/tmp/cpuinfo
sed 's/^MemAvailable:.*/MemAvailable:   16777216 kB/' /proc/meminfo >/tmp/meminfo
mount --bind /tmp/meminfo /proc/meminfo

mkdir -p "$KIT"
cp /e2e/lan-test.sh /e2e/seed.iso "$KIT/"
printf 'LAN_TEST_MODE=quick\nLAN_TEST_SETUP_MINUTES=150\nLAN_TEST_BOOT_MINUTES=40\n' >"$KIT/lan-test.env"
chown -R friend:friend "$KIT"
# Run the checks as friend so signals reach lan-test.sh itself, not sudo.
exec sudo -u friend -H env PATH="/opt/shim:$PATH" DCL_LAN_TEST_CPUINFO=/tmp/cpuinfo \
  bash /e2e/quick-tcg.sh --driver
fi


pass=0; failn=0
ok() { echo "  OK   $*"; pass=$((pass + 1)); }
bad() { echo "  FAIL $*"; failn=$((failn + 1)); }

one_run() {
  local label="$1" wait_min="$2" out="/tmp/$1.out"
  echo "== $label ($(date +%H:%M))"
  # Start lan-test.sh directly: a function + & would fork a subshell and the
  # TERM below would hit that subshell, not the app.
  bash "$KIT/lan-test.sh" >"$out" 2>&1 &
  local pid=$! start=$SECONDS
  while (( SECONDS - start < wait_min * 60 )); do
    grep -q "The test computer is ready" "$out" && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 10
  done
  local mins=$(( (SECONDS - start) / 60 ))
  if ! grep -q "The test computer is ready" "$out"; then
    bad "$label: not ready after ${mins} min"
    tail -n 60 "$out"
    ls "$KIT"/lan-test-serial-*.log >/dev/null 2>&1 && tail -n 60 "$(ls -t "$KIT"/lan-test-serial-*.log | head -n1)"
    kill -TERM "$pid" 2>/dev/null; wait "$pid"
    cat "$out" | tail -n 30
    return
  fi
  ok "$label: ready after ${mins} min"
  local link pw
  link=$(grep -o 'https://[0-9.]*:[0-9]*' "$out" | head -n1)
  pw=$(sed -n 's/^ *Password: *//p' "$out")
  [[ "$(curl -sk -o /dev/null -w '%{http_code}' "$link/")" == 401 ]] && ok "no password -> 401" || bad "no password"
  [[ "$(curl -sk -o /dev/null -w '%{http_code}' -u ubuntu:wrong "$link/")" == 401 ]] && ok "wrong password -> 401" || bad "wrong password"
  local body
  body="$(curl -sk -u "ubuntu:$pw" "$link/")"
  [[ "$(curl -sk -o /dev/null -w '%{http_code}' -u "ubuntu:$pw" "$link/")" == 200 ]] && ok "right password -> 200" || bad "right password"
  grep -qi kasm <<<"$body" && ok "page is the KasmVNC web client" || bad "page body: ${body:0:200}"
  grep -q "DCL-QUICK-SETUP-OK" "$KIT"/lan-test-serial-*.log 2>/dev/null && ok "serial shows setup OK" || true
  kill -TERM "$pid"; wait "$pid"
  sleep 3
  pgrep -f qemu-system-x86_64 >/dev/null && bad "QEMU still running" || ok "VM stopped"
  grep -q "Result: PASS (quick mode)" "$(ls -t "$KIT"/lan-test-report-*.txt | head -n1)" && ok "report PASS" || { bad "report"; tail -n 15 "$out"; }
}

one_run first-boot 160
one_run second-boot 45
echo "setup-done marker: $(cat /home/friend/.cache/dcl-lan-test/quick/setup-done 2>/dev/null | cut -c1-12)"
echo "Second run boot line: $(grep 'Desktop answered' /tmp/second-boot.out)"
echo
echo "RESULT: $pass passed, $failn failed"
[[ $failn -eq 0 ]]
