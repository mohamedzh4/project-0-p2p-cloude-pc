#!/usr/bin/env bash
# Shared guest packages. Runs inside the Packer VM as root.
# Tailscale is the *client* used with self-hosted Headscale (not Tailscale SaaS).
# rclone is the guest mount tool for renterd S3 (not Storj). Neither is started here.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive

# Packer SSH is often ready while cloud-init still holds apt locks.
if command -v cloud-init >/dev/null 2>&1; then
  cloud-init status --wait || true
fi

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
apt-get install -y --no-install-recommends \
  ca-certificates \
  curl \
  fuse3 \
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
