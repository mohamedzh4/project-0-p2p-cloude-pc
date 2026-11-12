#!/usr/bin/env bash
# Install a direct KasmVNC + XFCE browser desktop in the Standard guest.
#
# Authentication is deliberately disabled only on the mesh listener. Public
# access is authenticated by the DCL gateway's short-lived session token, and
# Headscale ACLs permit the gateway/assigned roles to reach guest port 8443.
#
# LAN test mode: when the host passes QEMU fw_cfg opt/org.dcl/lan-password,
# KasmVNC turns basic auth ON (user `ubuntu`, that password) because the
# listener is then reachable from a shared Wi-Fi without the gateway.
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

curl -fL -o /tmp/kasmvncserver.deb "${KASM_VNC_DEB_URL}"
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

# Root pre-start: fw_cfg entries are root-only (0400). A LAN password turns
# on KasmVNC basic auth; no password keeps the mesh/gateway behaviour.
# KasmVNC always needs a user with write access, otherwise `vncserver`
# stops at an interactive "create a user?" prompt that systemd cannot
# answer. Without a LAN password the user gets a random throwaway password
# (basic auth stays disabled, so it is never asked for).
cat >/usr/local/sbin/dcl-kasmvnc-auth <<'EOF'
#!/bin/sh
set -eu
FWCFG=/sys/firmware/qemu_fw_cfg/by_name/opt/org.dcl/lan-password/raw
MARKER=/run/dcl-kasmvnc/basic-auth
rm -f "$MARKER" /home/ubuntu/.kasmpasswd
modprobe -q qemu_fw_cfg 2>/dev/null || true
password=""
if [ -r "$FWCFG" ]; then
  password="$(tr -d '\r\n' <"$FWCFG")"
fi
lan=1
if [ -z "$password" ]; then
  lan=0
  password="$(head -c 48 /dev/urandom | base64 | tr -dc 'A-Za-z0-9')"
fi
passwd_bin="$(command -v kasmvncpasswd || command -v vncpasswd)"
printf '%s\n%s\n' "$password" "$password" |
  runuser -u ubuntu -- env HOME=/home/ubuntu "$passwd_bin" -u ubuntu -wo >/dev/null
if [ "$lan" -eq 1 ]; then
  touch "$MARKER"
  chown ubuntu:ubuntu "$MARKER"
  echo "dcl-kasmvnc: LAN password from fw_cfg; basic auth on"
else
  echo "dcl-kasmvnc: no LAN password; basic auth off (mesh/gateway mode)"
fi
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
After=network-online.target
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
