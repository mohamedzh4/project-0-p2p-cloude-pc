#!/usr/bin/env bash
# Install a direct KasmVNC + XFCE browser desktop in the Standard guest.
#
# The desktop asks for a password whenever the VM runs a session:
#
# - Session mode (the product): the Matchmaker makes a password per session.
#   dcl-guest-join stores it in /run/dcl/desktop-password before the guest
#   joins the mesh, and KasmVNC basic auth is ON (user `ubuntu`). Only the
#   gateway is told the password and sends it for the renter. Without this,
#   any machine on the mesh that can reach port 8443 (every Host Agent can)
#   could open a renter's desktop.
# - A session VM that never received its password stays locked: basic auth
#   ON with a random password nobody knows.
# - LAN test mode: the host passes SMBIOS OEM string dcl-lan-password=...
#   (QEMU -smbios type=11; fw_cfg opt/org.dcl/lan-password as a fallback).
#   Basic auth ON with that password, because the listener is then reachable
#   from a shared Wi-Fi without the gateway. SMBIOS first: Ubuntu's cloud
#   kernel has no qemu_fw_cfg module.
# - Only a VM with no session and no LAN password (image build, manual boot)
#   runs with basic auth off.
set -euo pipefail

if [[ "${LAB_TIER:-}" == "pro" ]]; then
  echo "provision-kasm.sh is Standard-only" >&2
  exit 1
fi

: "${KASM_VNC_DEB_URL:?KASM_VNC_DEB_URL is required}"
: "${KASM_VNC_DEB_SHA256:?KASM_VNC_DEB_SHA256 is required}"

export DEBIAN_FRONTEND=noninteractive
apt-get update -y
apt-get install -y --no-install-recommends \
  dbus-x11 \
  xfce4 \
  xfce4-terminal \
  xfonts-base

curl -fsSL --retry 5 --retry-delay 5 --retry-all-errors --connect-timeout 20 \
  -o /tmp/kasmvncserver.deb "${KASM_VNC_DEB_URL}"
echo "${KASM_VNC_DEB_SHA256}  /tmp/kasmvncserver.deb" | sha256sum -c -
apt-get install -y /tmp/kasmvncserver.deb
rm -f /tmp/kasmvncserver.deb

usermod -aG ssl-cert ubuntu
install -d -m 0700 -o ubuntu -g ubuntu /home/ubuntu/.vnc
cat >/home/ubuntu/.vnc/xstartup <<'EOF'
#!/bin/sh
unset SESSION_MANAGER
unset DBUS_SESSION_BUS_ADDRESS
exec dbus-run-session startxfce4
EOF
chown ubuntu:ubuntu /home/ubuntu/.vnc/xstartup
chmod 0700 /home/ubuntu/.vnc/xstartup
# Without this marker vncserver stops at an interactive "choose Desktop
# Environment" prompt (systemd cannot answer it). xstartup above is XFCE.
touch /home/ubuntu/.vnc/.de-was-selected
chown ubuntu:ubuntu /home/ubuntu/.vnc/.de-was-selected

cat >/home/ubuntu/.vnc/kasmvnc.yaml <<'EOF'
network:
  interface: 0.0.0.0
  websocket_port: 8443
  use_ipv4: true
  use_ipv6: true
  ssl:
    require_ssl: true
server:
  auto_shutdown:
    no_user_session_timeout: never
    active_user_session_timeout: never
    inactive_user_session_timeout: never
EOF
chown ubuntu:ubuntu /home/ubuntu/.vnc/kasmvnc.yaml
chmod 0600 /home/ubuntu/.vnc/kasmvnc.yaml

# Root pre-start (dmidecode and fw_cfg are root-only). Decides the desktop
# password; the four cases are described at the top of this file.
# KasmVNC always needs a user with write access, otherwise `vncserver`
# stops at an interactive "create a user?" prompt that systemd cannot
# answer. So a password is always set, even when basic auth stays off.
cat >/usr/local/sbin/dcl-kasmvnc-auth <<'EOF'
#!/bin/sh
set -eu
FWCFG=/sys/firmware/qemu_fw_cfg/by_name/opt/org.dcl/lan-password/raw
SESSION_PASSWORD=/run/dcl/desktop-password
SESSION_MARK=/run/dcl/session-vm
BOOTSTRAP_FWCFG=/sys/firmware/qemu_fw_cfg/by_name/opt/org.dcl/bootstrap/raw
MARKER=/run/dcl-kasmvnc/basic-auth
rm -f "$MARKER" /home/ubuntu/.kasmpasswd
modprobe -q qemu_fw_cfg 2>/dev/null || true

oem_strings() {
  if command -v dmidecode >/dev/null 2>&1; then
    dmidecode -t 11 2>/dev/null || true
  fi
}

# True when this VM was started for a renter's session.
session_vm() {
  [ -e "$SESSION_MARK" ] && return 0
  grep -q 'dcl\.bootstrap=' /proc/cmdline 2>/dev/null && return 0
  oem_strings | grep -q 'dcl\.bootstrap=' && return 0
  [ -r "$BOOTSTRAP_FWCFG" ] && return 0
  return 1
}

mode=open
password="$(oem_strings |
  sed -n 's/^[[:space:]]*String [0-9]*: dcl-lan-password=//p' | head -n 1 | tr -d '\r\n')"
if [ -z "$password" ] && [ -r "$FWCFG" ]; then
  password="$(tr -d '\r\n' <"$FWCFG")"
fi
if [ -n "$password" ]; then
  mode=lan
elif [ -s "$SESSION_PASSWORD" ]; then
  password="$(tr -d '\r\n' <"$SESSION_PASSWORD")"
  mode=session
fi
if [ -z "$password" ]; then
  password="$(head -c 48 /dev/urandom | base64 | tr -dc 'A-Za-z0-9')"
  if session_vm; then
    mode=locked
  fi
fi
passwd_bin="$(command -v kasmvncpasswd || command -v vncpasswd)"
printf '%s\n%s\n' "$password" "$password" |
  runuser -u ubuntu -- env HOME=/home/ubuntu "$passwd_bin" -u ubuntu -wo >/dev/null
if [ "$mode" != open ]; then
  touch "$MARKER"
  chown ubuntu:ubuntu "$MARKER"
fi
case "$mode" in
  lan) echo "dcl-kasmvnc: LAN password from the host; basic auth on" ;;
  session) echo "dcl-kasmvnc: session password from the Matchmaker; basic auth on" ;;
  locked) echo "dcl-kasmvnc: session VM without a desktop password; locked (basic auth on)" ;;
  open) echo "dcl-kasmvnc: no session and no LAN password; basic auth off" ;;
esac
EOF
chmod 0755 /usr/local/sbin/dcl-kasmvnc-auth

cat >/usr/local/sbin/dcl-kasmvnc-start <<'EOF'
#!/bin/sh
set -eu
if [ -e /run/dcl-kasmvnc/basic-auth ]; then
  exec /usr/bin/vncserver :1 -fg -geometry 1920x1080 -depth 24
fi
exec /usr/bin/vncserver :1 -fg -disableBasicAuth -geometry 1920x1080 -depth 24
EOF
chmod 0755 /usr/local/sbin/dcl-kasmvnc-start

cat >/etc/systemd/system/dcl-kasmvnc.service <<'EOF'
[Unit]
Description=Distributed Compute Lab browser desktop
# After the join: it stores the session's desktop password and mounts the
# renter's files. Ordering only; the desktop still starts if the join fails
# (then it is locked, see dcl-kasmvnc-auth).
After=network-online.target dcl-guest-join.service
Wants=network-online.target

[Service]
Type=simple
User=ubuntu
Group=ubuntu
SupplementaryGroups=ssl-cert
Environment=HOME=/home/ubuntu
RuntimeDirectory=dcl-kasmvnc
ExecStartPre=+/usr/local/sbin/dcl-kasmvnc-auth
ExecStart=/usr/local/sbin/dcl-kasmvnc-start
ExecStop=/usr/bin/vncserver -kill :1
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable dcl-kasmvnc.service
