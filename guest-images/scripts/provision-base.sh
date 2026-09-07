#!/usr/bin/env bash
# Shared guest packages. Runs inside the Packer VM as root.
# Tailscale is the *client* used with self-hosted Headscale (not Tailscale SaaS).
# rclone is the guest mount tool for renterd S3 (not Storj). Neither is started here.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive

apt-get update
apt-get install -y --no-install-recommends \
  ca-certificates \
  curl \
  gnupg \
  python3 \
  qemu-guest-agent \
  rclone \
  sudo

# Official Tailscale client install (login-server is supplied at session start).
# https://tailscale.com/kb/1031/install-linux
if ! command -v tailscale >/dev/null 2>&1; then
  curl -fsSL https://tailscale.com/install.sh | sh
fi

# Do not join Headscale or rclone-mount during image build.
# Host Agent / Matchmaker inject locators at session time.

cloud-init clean --logs --seed || true
apt-get clean
rm -rf /var/lib/apt/lists/*
