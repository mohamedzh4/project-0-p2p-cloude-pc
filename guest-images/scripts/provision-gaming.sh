#!/usr/bin/env bash
# Pro guest: make it a gaming PC. Runs in the Packer VM (no GPU there), so
# drivers are installed for every vendor and the right one loads on the
# host's GPU at boot:
#   - NVIDIA driver (kernel module, GL, Vulkan, NVENC; 32-bit GL for games)
#   - Mesa for AMD/Intel (Vulkan RADV/ANV, VAAPI encode)
#   - Steam with Proton (Windows games), Wine + Lutris for non-Steam games,
#     GameMode, MangoHud
#   - XFCE logs in by itself, never locks or blanks the screen
#   - a virtual sound card that games play into and the streamer records
#   - a virtual monitor for a GPU with nothing plugged in (dcl-display-setup)
#
# Note: installing Steam accepts the Steam installer's licence prompt
# (debconf below). Building the image means you accept it.
set -euo pipefail

if [[ "${LAB_TIER:-}" != "pro" ]]; then
  echo "provision-gaming.sh is Pro-only" >&2
  exit 1
fi

export DEBIAN_FRONTEND=noninteractive
apt_update() {
  local retry=0
  until apt-get update; do
    retry=$((retry + 1))
    if (( retry >= 36 )); then
      echo "apt-get update failed after ${retry} attempts" >&2
      exit 1
    fi
    sleep 10
  done
}

# 32-bit libraries (most Windows games under Proton are 32-bit) and the
# multiverse component (Steam, Intel's media driver).
dpkg --add-architecture i386
if command -v add-apt-repository >/dev/null 2>&1; then
  add-apt-repository -y multiverse
else
  sed -i 's/^Components: main universe restricted$/Components: main universe restricted multiverse/' \
    /etc/apt/sources.list.d/ubuntu.sources
fi
apt_update

# Kernel headers so the NVIDIA module builds for the image's kernel.
apt-get install -y --no-install-recommends linux-generic linux-headers-generic

# NVIDIA: pinned package if given, else the newest non-server proprietary
# driver in the archive. The proprietary one (not -open) also covers GTX 10-series.
nv="${NVIDIA_DRIVER_PACKAGE:-}"
if [[ -z "$nv" ]]; then
  nv="$(apt-cache pkgnames nvidia-driver- | grep -E '^nvidia-driver-[0-9]+$' | sort -V | tail -1)"
fi
[[ -n "$nv" ]] || { echo "no nvidia-driver package found" >&2; exit 1; }
nv_ver="${nv##*-}"
echo "installing $nv"
apt-get install -y "$nv" "libnvidia-gl-${nv_ver}:i386" "libnvidia-encode-${nv_ver}"

# AMD / Intel: Mesa GL + Vulkan (64 and 32-bit) and VAAPI encoders.
apt-get install -y --no-install-recommends \
  mesa-vulkan-drivers mesa-vulkan-drivers:i386 \
  libgl1-mesa-dri libgl1-mesa-dri:i386 libglx-mesa0:i386 \
  mesa-va-drivers intel-media-va-driver-non-free \
  vulkan-tools mesa-utils

# Steam (with Proton), Wine, Lutris, GameMode, MangoHud.
echo "steam steam/question select I AGREE" | debconf-set-selections
echo "steam steam/license note ''" | debconf-set-selections
apt-get install -y steam-installer
apt-get install -y --no-install-recommends \
  wine64 wine32:i386 winetricks lutris \
  gamemode mangohud \
  pipewire pipewire-pulse wireplumber pulseaudio-utils \
  x11-xserver-utils xdotool python3

# Desktop: log in as ubuntu at boot, no lock, no blanking.
mkdir -p /etc/lightdm/lightdm.conf.d
cat > /etc/lightdm/lightdm.conf.d/50-dcl-autologin.conf <<'EOF'
[Seat:*]
autologin-user=ubuntu
autologin-user-timeout=0
user-session=xfce
EOF
apt-get purge -y light-locker xfce4-screensaver 2>/dev/null || true
mkdir -p /etc/xdg/autostart
cat > /etc/xdg/autostart/dcl-no-blank.desktop <<'EOF'
[Desktop Entry]
Type=Application
Name=DCL: keep the screen on
Exec=sh -c 'xset s off -dpms s noblank'
NoDisplay=true
EOF

# Sound: a virtual sink, preferred over any real output, that games play
# into; dcl-stream-server records its monitor (dcl_game.monitor).
mkdir -p /etc/pipewire/pipewire.conf.d
cat > /etc/pipewire/pipewire.conf.d/50-dcl-game-sink.conf <<'EOF'
context.objects = [
  { factory = adapter
    args = {
      factory.name     = support.null-audio-sink
      node.name        = "dcl_game"
      node.description = "DCL Stream"
      media.class      = Audio/Sink
      audio.position   = [ FL FR ]
      audio.rate       = 48000
      priority.session = 2000
      priority.driver  = 2000
    }
  }
]
EOF

# Virtual monitor for a GPU with nothing plugged in.
install -d /usr/lib/firmware/edid
python3 /tmp/make_edid.py /usr/lib/firmware/edid/dcl-gaming.bin
install -m 0755 /tmp/dcl-display-setup.py /usr/local/sbin/dcl-display-setup
cat > /etc/systemd/system/dcl-display-setup.service <<'EOF'
[Unit]
Description=DCL: give the passthrough GPU a screen
Before=display-manager.service
After=systemd-udev-settle.service sys-kernel-debug.mount
Wants=sys-kernel-debug.mount

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/dcl-display-setup

[Install]
WantedBy=graphical.target
EOF
systemctl enable dcl-display-setup.service

# NVIDIA KMS (needed for PRIME and later zero-copy capture).
if [[ -f /etc/default/grub ]]; then
  sed -i 's/^GRUB_CMDLINE_LINUX_DEFAULT="\(.*\)"/GRUB_CMDLINE_LINUX_DEFAULT="\1 nvidia-drm.modeset=1"/' /etc/default/grub
  update-grub || true
fi

systemctl set-default graphical.target
apt-get clean
rm -rf /var/lib/apt/lists/*
