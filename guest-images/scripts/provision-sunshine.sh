#!/usr/bin/env bash
# Install Sunshine *inside the guest* (Pro streamer). Moonlight is the client.
# Debian/Ubuntu path from LizardByte:
#   https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2getting__started.html
#   sudo dpkg -i ./sunshine-ubuntu-24.04-{arch}.deb
# User service enable:
#   systemctl --user --now enable app-dev.lizardbyte.app.Sunshine
#   (aliased to sunshine.service)
set -euo pipefail

if [[ "${LAB_TIER:-}" == "standard" ]]; then
  echo "provision-sunshine.sh is Pro-only" >&2
  exit 1
fi

: "${SUNSHINE_DEB_URL:?SUNSHINE_DEB_URL is required}"

export DEBIAN_FRONTEND=noninteractive

retry=0
until apt-get update; do
  retry=$((retry + 1))
  if (( retry >= 36 )); then
    echo "apt-get update failed after ${retry} attempts" >&2
    exit 1
  fi
  echo "apt-get update retry ${retry}/36 (waiting for lock)" >&2
  sleep 10
done

# A display session so Sunshine has something to capture. NVIDIA/AMD guest
# drivers are *not* baked here — the Packer VM has no passthrough GPU.
# On first Pro boot with VFIO/DDA, install the vendor driver in the guest.
apt-get install -y --no-install-recommends \
  xfce4 \
  xfce4-goodies \
  lightdm \
  dbus-user-session \
  xserver-xorg

cd /tmp
curl -fL -o sunshine.deb "${SUNSHINE_DEB_URL}"
# Documented install is dpkg; -f pulls encoder dependencies if the .deb needs them.
dpkg -i ./sunshine.deb || apt-get install -y -f
rm -f /tmp/sunshine.deb

# Graphical target + linger so the user unit starts without a console login.
systemctl set-default graphical.target
loginctl enable-linger ubuntu

# Official unit name (XDG portal) plus the sunshine.service alias.
sudo -u ubuntu XDG_RUNTIME_DIR="/run/user/$(id -u ubuntu)" \
  systemctl --user --now enable app-dev.lizardbyte.app.Sunshine || \
sudo -u ubuntu XDG_RUNTIME_DIR="/run/user/$(id -u ubuntu)" \
  systemctl --user --now enable sunshine.service

apt-get clean
rm -rf /var/lib/apt/lists/*
