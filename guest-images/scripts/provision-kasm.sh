#!/usr/bin/env bash
# Install Kasm Workspaces *inside the guest* (Standard streamer).
# Flags match the current Kasm single-server docs:
#   https://kasm.com/docs/latest/install/single_server_install.html
#   https://docs.kasm.com/docs/tutorials/install/single-server-install
# --accept-eula / --admin-password / --user-password are the unattended path
# used by Kasm's own installer (also listed on the 1.17+ pages).
set -euo pipefail

if [[ "${LAB_TIER:-}" == "pro" ]]; then
  echo "provision-kasm.sh is Standard-only" >&2
  exit 1
fi

: "${KASM_TARBALL_URL:?KASM_TARBALL_URL is required}"
: "${KASM_TARBALL_SHA256:?KASM_TARBALL_SHA256 is required}"
: "${DCL_KASM_ADMIN_PASSWORD:?DCL_KASM_ADMIN_PASSWORD is required (do not commit)}"
: "${DCL_KASM_USER_PASSWORD:?DCL_KASM_USER_PASSWORD is required (do not commit)}"

export DEBIAN_FRONTEND=noninteractive
cd /tmp
curl -fL -o kasm_release.tar.gz "${KASM_TARBALL_URL}"
echo "${KASM_TARBALL_SHA256}  kasm_release.tar.gz" | sha256sum -c -
tar -xf kasm_release.tar.gz

# Nested Docker is installed by Kasm's install.sh. This needs overlayfs +
# cgroup support in the kernel that later boots the disk (Firecracker kernel
# is a separate operator input — see guest-images/README.md).
sudo bash kasm_release/install.sh \
  --accept-eula \
  --admin-password "${DCL_KASM_ADMIN_PASSWORD}" \
  --user-password "${DCL_KASM_USER_PASSWORD}" \
  --swap-size 4096

rm -f /tmp/kasm_release.tar.gz
rm -rf /tmp/kasm_release
