#!/usr/bin/env bash
# shellcheck disable=SC2015,SC2012  # test harness: ok()/bad() always return 0
# Runs as user "friend" inside the container.
set -uo pipefail
KIT=/home/friend/kit
pass=0; failn=0
ok() { echo "  OK   $*"; pass=$((pass+1)); }
bad() { echo "  FAIL $*"; failn=$((failn+1)); }
reset_kit() { rm -f "$KIT"/lan-test-report-*.txt "$KIT"/lan-test-serial-*.log; }
latest_report() { ls -t "$KIT"/lan-test-report-*.txt | head -n1; }

echo "== A. --check"
reset_kit
bash "$KIT/lan-test.sh" --check >/tmp/a.out 2>&1; rc=$?
[[ $rc -eq 0 ]] && ok "exit 0" || { bad "exit $rc"; cat /tmp/a.out; }
grep -q "Result: CHECK PASSED" "$(latest_report)" && ok "report says CHECK PASSED" || bad "report"

echo "== B. full run"
reset_kit
bash "$KIT/lan-test.sh" >/tmp/b.out 2>&1 &
pid=$!
for _ in $(seq 1 120); do grep -q "The test computer is ready" /tmp/b.out && break; sleep 1; done
if grep -q "The test computer is ready" /tmp/b.out; then ok "ready banner"; else bad "no banner"; cat /tmp/b.out; fi
link=$(grep -o 'https://[0-9.]*:[0-9]*' /tmp/b.out | head -n1)
pw=$(sed -n 's/^ *Password: *//p' /tmp/b.out)
echo "  link=$link pwlen=${#pw}"
[[ ${#pw} -eq 12 ]] && ok "12-char password" || bad "password length ${#pw}"
[[ "$link" != *127.0.0.1* && -n "$link" ]] && ok "link uses LAN address" || bad "link $link"
c1=$(curl -sk -o /dev/null -w '%{http_code}' "$link/"); [[ $c1 == 401 ]] && ok "no password -> 401" || bad "no password -> $c1"
c2=$(curl -sk -o /dev/null -w '%{http_code}' -u "ubuntu:wrongpass" "$link/"); [[ $c2 == 401 ]] && ok "wrong password -> 401" || bad "wrong -> $c2"
c3=$(curl -sk -o /dev/null -w '%{http_code}' -u "ubuntu:$pw" "$link/"); [[ $c3 == 200 ]] && ok "right password -> 200" || bad "right -> $c3"
python3 - <<'PY' && ok "argv has kvm/snapshot/hostfwd/smbios/virtio" || bad "argv"
import json; a = json.load(open("/tmp/qemu-argv.json")); s = " ".join(a)
for f in ["-enable-kvm", "-snapshot", "-machine q35", "-cpu host", "-name dcl-lan-test",
          "if=virtio,format=raw", "hostfwd=tcp:0.0.0.0:8443-:8443", "type=11,path=",
          "virtio-net-pci,netdev=net0", "-display none"]:
    assert f in s, f
PY
pwfile=$(python3 -c 'import json;a=json.load(open("/tmp/qemu-argv.json"));print([x for x in a if x.startswith("type=11,path=")][0].split("path=")[1])')
[[ ! -e "$pwfile" ]] && ok "password file removed after boot" || bad "password file still exists"
kill -TERM $pid; wait $pid
sleep 1
pgrep -f fake-qemu >/dev/null && bad "fake qemu still running" || ok "VM stopped on Ctrl+C/TERM"
r=$(latest_report)
grep -q "Result: PASS (signed mode): desktop reachable" "$r" && ok "report PASS" || { bad "report result"; cat "$r"; }
grep -q -- "$pw" "$r" && bad "password leaked into report" || ok "password not in report"
grep -q "Desktop answered after" "$r" && ok "boot time recorded" || bad "no boot time"

echo "== C. wrong digest"
reset_kit
cp "$KIT/lan-test.env" /tmp/env.bak
sed -i 's/^IMAGE_STANDARD_DIGEST=.*/IMAGE_STANDARD_DIGEST='"$(printf 'b%.0s' {1..64})"'/' "$KIT/lan-test.env"
rm -f /home/friend/.cache/dcl-host-agent/images/standard/rootfs.img
bash "$KIT/lan-test.sh" >/tmp/c.out 2>&1; rc=$?
[[ $rc -ne 0 ]] && ok "non-zero exit" || bad "exit 0"
grep -q "SHA-256 does not match" /tmp/c.out && ok "says digest mismatch" || { bad "message"; cat /tmp/c.out; }
[[ ! -e /home/friend/.cache/dcl-host-agent/images/standard/rootfs.img ]] && ok "untrusted image deleted" || bad "image kept"
[[ ! -e /tmp/qemu-argv.json.c ]] && ok "no VM started" || true
cp /tmp/env.bak "$KIT/lan-test.env"

echo "== D. QEMU fails to start"
reset_kit
FAKE_QEMU_CRASH=1 bash "$KIT/lan-test.sh" >/tmp/d.out 2>&1; rc=$?
[[ $rc -ne 0 ]] && ok "non-zero exit" || bad "exit 0"
grep -q "failed to initialize kvm" "$(latest_report)" && ok "QEMU error copied to report" || { bad "no qemu error in report"; cat /tmp/d.out; }

echo "== E. port already used"
reset_kit
python3 -m http.server 8443 >/dev/null 2>&1 & hp=$!; sleep 1
bash "$KIT/lan-test.sh" --check >/tmp/e.out 2>&1; rc=$?
kill $hp
[[ $rc -ne 0 ]] && grep -q "Port 8443 is already in use" /tmp/e.out && ok "busy port refused" || { bad "busy port"; cat /tmp/e.out; }

echo "== F. unknown key in lan-test.env"
reset_kit
echo "EVIL=1" >> "$KIT/lan-test.env"
bash "$KIT/lan-test.sh" --check >/tmp/f.out 2>&1; rc=$?
cp /tmp/env.bak "$KIT/lan-test.env"
[[ $rc -ne 0 ]] && grep -q "Unsupported key" /tmp/f.out && ok "unknown key refused" || bad "unknown key"

echo
echo "RESULT: $pass passed, $failn failed"
[[ $failn -eq 0 ]]
