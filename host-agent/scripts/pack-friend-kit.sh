#!/usr/bin/env bash
# Build the folder you send the Windows volunteer. Does not compile Host Agent
# (this Mac cannot). Put a Windows NSIS exe next to this script or pass its path.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="$(cd "$ROOT/.." && pwd)"
OUT="${FRIEND_KIT_DIR:-$ROOT/dist-friend}"
FRIEND_ENV="${FRIEND_ENV_FILE:-$ROOT/.env.friend}"
INSTALLER="${HOST_AGENT_INSTALLER:-}"
QCOW="${IMAGE_QCOW2:-}"

usage() {
  cat <<'EOF'
Usage: ./scripts/pack-friend-kit.sh [host-agent-nsis.exe]

Creates host-agent/dist-friend/ with:
  setup.cmd                 double-click this
  windows-host-setup.ps1
  friend.env                bootstrap token + image pins (gitignored)
  host-agent-setup.exe      if you passed a Windows NSIS build
  rootfs.qcow2              only if IMAGE_QCOW2 is set (large)

The volunteer: double-click setup.cmd, click Yes on UAC. BIOS, CGNAT, and
router forwarding still cannot be done by software.

This Mac cannot produce host-agent-setup.exe. Build on Windows or download
the GitHub Actions artifact (.github/workflows/host-agent.yml).
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ -n "${1:-}" ]]; then
  INSTALLER="$1"
fi

if [[ ! -f "$FRIEND_ENV" ]]; then
  echo "missing $FRIEND_ENV (gitignored). It is created when Matchmaker pins are applied." >&2
  exit 1
fi

if [[ -z "$INSTALLER" ]]; then
  shopt -s nullglob
  candidates=(
    "$ROOT/src-tauri/target/release/bundle/nsis/"*.exe
    "$OUT/"host-agent-setup.exe
  )
  shopt -u nullglob
  for c in "${candidates[@]}"; do
    if [[ -f "$c" ]]; then
      INSTALLER="$c"
      break
    fi
  done
fi

if [[ -z "$QCOW" ]]; then
  : # do not auto-copy a 9 GiB disk onto this Mac
fi

rm -rf "$OUT"
mkdir -p "$OUT"
cp "$ROOT/scripts/windows-host-setup.ps1" "$OUT/windows-host-setup.ps1"
cp "$ROOT/scripts/setup.cmd" "$OUT/setup.cmd"
cp "$ROOT/scripts/start-tunnel.cmd" "$OUT/start-tunnel.cmd"
if [[ -f "$ROOT/.tunnel/dcl-tunnel.pem" ]]; then
  cp "$ROOT/.tunnel/dcl-tunnel.pem" "$OUT/tunnel.pem"
  chmod 600 "$OUT/tunnel.pem"
  echo "included reverse-SSH tunnel key"
fi

export _DCL_FRIEND_ENV="$FRIEND_ENV"
export _DCL_FRIEND_OUT="$OUT"
python3 - <<'PY'
from pathlib import Path
import os

friend = Path(os.environ["_DCL_FRIEND_ENV"]).read_text()
vals = {}
for line in friend.splitlines():
    if not line or line.startswith("#") or "=" not in line:
        continue
    k, _, v = line.partition("=")
    vals[k.strip()] = v.strip()

need = [
    "HOST_BOOTSTRAP_TOKEN",
    "IMAGE_SIGNING_PUBLIC_KEY",
    "IMAGE_STANDARD_DIGEST",
    "IMAGE_STANDARD_SIGNATURE",
]
missing = [k for k in need if not vals.get(k)]
if missing:
    raise SystemExit("friend env missing " + ",".join(missing))

qcow_url = vals.get("IMAGE_QCOW2_URL") or ""
installer_url = vals.get("HOST_AGENT_INSTALLER_URL") or ""
display = vals.get("HOST_DISPLAY_NAME") or ""
text = (
    f'HOST_BOOTSTRAP_TOKEN={vals["HOST_BOOTSTRAP_TOKEN"]}\n'
    f'MATCHMAKER_URL={vals.get("MATCHMAKER_URL") or "https://lab.example.com/api"}\n'
    f'HEADSCALE_LOGIN_SERVER={vals.get("HEADSCALE_LOGIN_SERVER") or "https://mesh.lab.example.com"}\n'
    f'IMAGE_SIGNING_PUBLIC_KEY={vals["IMAGE_SIGNING_PUBLIC_KEY"]}\n'
    f'IMAGE_STANDARD_DIGEST={vals["IMAGE_STANDARD_DIGEST"]}\n'
    f'IMAGE_STANDARD_SIGNATURE={vals["IMAGE_STANDARD_SIGNATURE"]}\n'
    f'HOST_DISPLAY_NAME={display}\n'
    f'HOST_AGENT_LISTEN_ADDR={vals.get("HOST_AGENT_LISTEN_ADDR") or "127.0.0.1:7842"}\n'
    f'HOST_AGENT_URL={vals.get("HOST_AGENT_URL") or "http://127.0.0.1:17842"}\n'
    f'TUNNEL_SSH={vals.get("TUNNEL_SSH") or "dcl-tunnel@203.0.113.10"}\n'
    f'TUNNEL_REMOTE_PORT={vals.get("TUNNEL_REMOTE_PORT") or "17842"}\n'
    f'IMAGE_QCOW2_URL={qcow_url}\n'
    f'HOST_AGENT_INSTALLER_URL={installer_url}\n'
)
out = Path(os.environ["_DCL_FRIEND_OUT"]) / "friend.env"
out.write_text(text, encoding="ascii")
out.chmod(0o600)
print("wrote friend.env token_len", len(vals["HOST_BOOTSTRAP_TOKEN"]))
PY

if [[ -n "$INSTALLER" ]]; then
  cp "$INSTALLER" "$OUT/host-agent-setup.exe"
  echo "included Host Agent installer"
else
  echo "WARNING: no host-agent-setup.exe. Friend setup will stop until you add the Windows NSIS build." >&2
fi

if [[ -n "$QCOW" ]]; then
  echo "copying guest disk (large)..."
  cp "$QCOW" "$OUT/rootfs.qcow2"
else
  echo "NOTE: no rootfs.qcow2 in the kit. Put it in this folder or set IMAGE_QCOW2_URL in $FRIEND_ENV." >&2
fi

echo "kit ready: $OUT"
echo "send that folder (zip/Drive). Friend double-clicks setup.cmd as themselves; UAC Yes is the main click."
