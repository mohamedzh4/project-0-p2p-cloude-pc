#!/usr/bin/env bash
# Docker checks for the LAN test kit (no KVM needed, works on a Mac).
#   1. lan-test.sh end to end against a fake QEMU (22 checks)
#   2. guest KasmVNC password helpers against the real pinned KasmVNC .deb
# Files are streamed with tar, so Docker file sharing is not required.
set -euo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd -- "$HERE/../../../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/e2e"
cp "$HERE"/fake-qemu "$HERE"/driver.sh "$HERE"/setup.sh "$HERE"/kasm-auth.sh "$WORK/e2e/"
cp "$REPO/host-agent/scripts/lan-test/lan-test.sh" "$WORK/e2e/"
cp "$REPO/guest-images/scripts/provision-kasm.sh" "$WORK/e2e/"

echo "### 1. lan-test.sh end to end (fake QEMU)"
tar cf - -C "$WORK" e2e | docker run -i --rm --privileged --platform linux/amd64 ubuntu:24.04 \
  bash -c 'tar xf - -C / 2>/dev/null; bash /e2e/setup.sh' 2>&1 | grep -v debconf | sed -n '/^setup done/,$p'

echo
echo "### 2. KasmVNC password helpers (real KasmVNC)"
out="$(tar cf - -C "$WORK" e2e | docker run -i --rm --platform linux/amd64 ubuntu:24.04 \
  bash -c 'tar xf - -C / 2>/dev/null; bash /e2e/kasm-auth.sh' 2>&1 | grep -v debconf)"
printf '%s\n' "$out" | sed -n '/=== case 1/,$p'
if ! printf '%s\n' "$out" | grep -q "right pass   -> 200" ||
  ! printf '%s\n' "$out" | grep -q "wrong pass   -> 401" ||
  ! printf '%s\n' "$out" | sed -n '/=== case 2/,/=== case 3/p' | grep -q "no auth      -> 200" ||
  ! printf '%s\n' "$out" | sed -n '/=== case 3/,/=== case 4/p' | grep -q "no auth      -> 401" ||
  ! printf '%s\n' "$out" | sed -n '/=== case 3/,/=== case 4/p' | grep -q "right pass   -> 200" ||
  ! printf '%s\n' "$out" | sed -n '/=== case 4/,$p' | grep -q "old pass     -> 401"; then
  echo "KasmVNC check FAILED"
  exit 1
fi
echo "KasmVNC check passed"
