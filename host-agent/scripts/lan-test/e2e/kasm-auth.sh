#!/usr/bin/env bash
set -uo pipefail
export DEBIAN_FRONTEND=noninteractive
URL=https://github.com/kasmtech/KasmVNC/releases/download/v1.3.4/kasmvncserver_noble_1.3.4_amd64.deb
SHA=b222f104a839b1ec6ac4a28f5f2876463204e3c2ba3724a31c3fb59e1a780740
apt-get update -qq >/dev/null && apt-get install -y -qq curl ca-certificates ssl-cert util-linux >/dev/null
curl -fsSL -o /tmp/k.deb "$URL" && echo "$SHA  /tmp/k.deb" | sha256sum -c - || exit 1
apt-get install -y -qq /tmp/k.deb >/dev/null 2>&1 || { echo "deb install failed"; exit 1; }
useradd -m ubuntu; usermod -aG ssl-cert ubuntu
P=/e2e/provision-kasm.sh
extract() { awk -v start="$1" '$0==start{f=1;next} f&&$0=="EOF"{exit} f' "$P"; }
install -d -m 0700 -o ubuntu -g ubuntu /home/ubuntu/.vnc
extract "cat >/home/ubuntu/.vnc/kasmvnc.yaml <<'EOF'" >/home/ubuntu/.vnc/kasmvnc.yaml
printf '#!/bin/sh\nexec sleep infinity\n' >/home/ubuntu/.vnc/xstartup; chmod 755 /home/ubuntu/.vnc/xstartup
touch /home/ubuntu/.vnc/.de-was-selected
chown -R ubuntu:ubuntu /home/ubuntu/.vnc
extract "cat >/usr/local/sbin/dcl-kasmvnc-auth <<'EOF'" | sed 's#^FWCFG=.*#FWCFG=/tmp/fwcfg/raw#' >/usr/local/sbin/dcl-kasmvnc-auth
extract "cat >/usr/local/sbin/dcl-kasmvnc-start <<'EOF'" >/usr/local/sbin/dcl-kasmvnc-start
chmod 755 /usr/local/sbin/dcl-kasmvnc-*
mkdir -p /run/dcl-kasmvnc && chown ubuntu:ubuntu /run/dcl-kasmvnc
echo "--- helpers:"; head -3 /usr/local/sbin/dcl-kasmvnc-auth; cat /usr/local/sbin/dcl-kasmvnc-start
code() { curl -sk -o /dev/null -m 5 -w '%{http_code}' "$@" https://127.0.0.1:8443/; }
wait_up() { for _ in $(seq 1 60); do c=$(code); [[ $c =~ ^[1-5] ]] && return 0; sleep 1; done; return 1; }
stop_vnc() { runuser -u ubuntu -- env HOME=/home/ubuntu vncserver -kill :1 >/dev/null 2>&1; pkill -f Xvnc; sleep 2; }

echo "=== case 1: LAN password given"
mkdir -p /tmp/fwcfg && printf 'k7fm2x9qabcd' >/tmp/fwcfg/raw
/usr/local/sbin/dcl-kasmvnc-auth; echo "auth rc=$?"
ls -l /run/dcl-kasmvnc/ /home/ubuntu/.kasmpasswd; echo "kasmpasswd: $(cut -d: -f1,3 /home/ubuntu/.kasmpasswd)"
runuser -u ubuntu -- env HOME=/home/ubuntu /usr/local/sbin/dcl-kasmvnc-start >/tmp/vnc1.log 2>&1 &
wait_up || { echo "server never answered"; tail -30 /tmp/vnc1.log; ls /home/ubuntu/.vnc; tail -30 /home/ubuntu/.vnc/*.log; }
echo "no auth      -> $(code)"
echo "wrong pass   -> $(code -u ubuntu:nope)"
echo "right pass   -> $(code -u ubuntu:k7fm2x9qabcd)"
stop_vnc
echo "=== case 2: no LAN password (mesh/gateway mode)"
rm -f /tmp/fwcfg/raw
/usr/local/sbin/dcl-kasmvnc-auth; echo "auth rc=$?"; echo "marker: $(ls /run/dcl-kasmvnc/)"; echo "kasmpasswd: $(cut -d: -f1,3 /home/ubuntu/.kasmpasswd)"
runuser -u ubuntu -- env HOME=/home/ubuntu /usr/local/sbin/dcl-kasmvnc-start >/tmp/vnc2.log 2>&1 &
wait_up || { echo "server never answered"; tail -30 /tmp/vnc2.log; }
echo "no auth      -> $(code)"
stop_vnc
