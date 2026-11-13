#!/usr/bin/env bash
# Boot the real quick-mode guest under QEMU TCG in Docker (no KVM needed).
# Takes about an hour on an Apple-silicon Mac. See quick-tcg.sh.
set -euo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
KIT="$(cd -- "$HERE/.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
LAN_TEST_KIT_DIR="$WORK/kits" bash "$KIT/pack-lan-test-kit.sh" >/dev/null
mkdir -p "$WORK/e2e"
cp "$KIT/lan-test.sh" "$HERE/quick-tcg.sh" "$WORK/kits/seed.iso" "$WORK/e2e/"
tar cf - -C "$WORK" e2e | docker run -i --rm --privileged --platform linux/arm64 ubuntu:24.04 \
  bash -c 'tar xf - -C / 2>/dev/null; bash /e2e/quick-tcg.sh'
