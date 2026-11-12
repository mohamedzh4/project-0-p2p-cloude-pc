#!/usr/bin/env bash
# Install a direct KasmVNC + XFCE browser desktop in the Standard guest.
#
# Authentication is deliberately disabled only on the mesh listener. Public
# access is authenticated by the DCL gateway's short-lived session token, and
# Headscale ACLs permit the gateway/assigned roles to reach guest port 8443.
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
ExecStart=/usr/bin/vncserver :1 -fg -disableBasicAuth -geometry 1920x1080 -depth 24
ExecStop=/usr/bin/vncserver -kill :1
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable dcl-kasmvnc.service
