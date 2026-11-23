#!/usr/bin/env bash
# Remove what linux-host-setup.sh put on this PC (Ubuntu/Debian and Arch).
#
#   bash linux-host-uninstall.sh          Host Agent, its settings and host
#                                         token, the guest disk, autostart,
#                                         this PC's mesh login.
#   bash linux-host-uninstall.sh --all    Also QEMU, Tailscale and the kvm
#                                         group. Only if they were not on the
#                                         PC before the setup.
#
# Run as the desktop user that ran the setup, not with sudo.
set -euo pipefail

ALL=0
case "${1:-}" in
  "") ;;
  --all) ALL=1 ;;
  *)
    echo "Usage: bash linux-host-uninstall.sh [--all]" >&2
    exit 1
    ;;
esac

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "This runs on Linux only." >&2
  exit 1
fi
if [[ "${EUID}" -eq 0 ]]; then
  echo "Run this as the desktop user that ran the setup; it uses sudo when needed." >&2
  exit 1
fi
if command -v apt-get >/dev/null; then
  DISTRO=debian
elif command -v pacman >/dev/null; then
  DISTRO=arch
else
  echo "This supports Ubuntu/Debian and Arch only." >&2
  exit 1
fi

CONFIG_HOME="${XDG_CONFIG_HOME:-"$HOME/.config"}"
CACHE_HOME="${XDG_CACHE_HOME:-"$HOME/.cache"}"
DATA_HOME="${XDG_DATA_HOME:-"$HOME/.local/share"}"
APP_ID="com.distributedcomputelab.hostagent"

free_before="$(df -h --output=avail "$HOME" | tail -1 | tr -d ' ')"

step() { printf '\n== %s\n' "$1"; }

sudo -v

step "Stopping Host Agent and any guest VM it started"
pkill -x host-agent 2>/dev/null || true
pkill -f "$HOME/.local/bin/dcl-host-agent.AppImage" 2>/dev/null || true
# Only QEMU processes booting the project's guest disk.
pkill -f "qemu-system-x86_64.*dcl-host-agent" 2>/dev/null || true
sleep 2
pkill -9 -f "qemu-system-x86_64.*dcl-host-agent" 2>/dev/null || true

if systemctl --user list-unit-files dcl-host-tunnel.service >/dev/null 2>&1; then
  systemctl --user disable --now dcl-host-tunnel.service 2>/dev/null || true
fi
rm -f "$CONFIG_HOME/systemd/user/dcl-host-tunnel.service"
systemctl --user daemon-reload 2>/dev/null || true

step "Leaving the project's mesh"
if command -v tailscale >/dev/null; then
  sudo tailscale logout 2>/dev/null || true
  sudo tailscale set --operator= 2>/dev/null || true
fi

step "Removing Host Agent"
if [[ "$DISTRO" == debian ]] && dpkg -s host-agent >/dev/null 2>&1; then
  sudo apt-get purge -y host-agent
fi
rm -f "$HOME/.local/bin/dcl-host-agent.AppImage"
rm -f "$DATA_HOME/applications/dcl-host-agent.desktop"
rm -f "$CONFIG_HOME/autostart/dcl-host-agent.desktop"

step "Deleting settings, host token, logs and the guest disk"
# The config dir holds the host bootstrap token and this PC's host secret.
rm -rf "$CONFIG_HOME/$APP_ID"
rm -rf "$CACHE_HOME/dcl-host-agent"
rm -rf "$CACHE_HOME/$APP_ID" "$DATA_HOME/$APP_ID"

if [[ "$ALL" -eq 1 ]]; then
  step "Removing QEMU, Tailscale and the kvm group (--all)"
  if [[ "$DISTRO" == debian ]]; then
    sudo apt-get purge -y qemu-system-x86 qemu-utils tailscale || true
    sudo rm -f /etc/apt/sources.list.d/tailscale.list \
      /usr/share/keyrings/tailscale-archive-keyring.gpg
    sudo rm -rf /var/lib/tailscale
    echo "Other packages that are no longer needed (not removed; check first):"
    sudo apt-get -s autoremove | awk '/^Remv /{print "  " $2}' || true
  else
    for pkg in qemu-base fuse2 tailscale; do
      if pacman -Q "$pkg" >/dev/null 2>&1; then
        sudo systemctl disable --now tailscaled 2>/dev/null || true
        sudo pacman -Rns --noconfirm "$pkg" || echo "  kept $pkg (another package needs it)"
      fi
    done
    sudo rm -rf /var/lib/tailscale
  fi
  if id -nG "$USER" | tr ' ' '\n' | grep -qx kvm; then
    sudo gpasswd -d "$USER" kvm || true
  fi
fi

free_after="$(df -h --output=avail "$HOME" | tail -1 | tr -d ' ')"
step "Done"
echo "Free space in $HOME: $free_before before, $free_after now."
echo "Also delete the kit folder you ran the setup from (it holds friend.env)."
if [[ "$ALL" -eq 0 ]]; then
  echo "QEMU and Tailscale were left installed. Run with --all if the setup added them."
fi
