#!/usr/bin/env bash
# Runs inside scripts/linux-check.Dockerfile (or on any Ubuntu 24.04 with the
# same packages). Builds all features, runs the tests, then streams a real
# X screen through FFmpeg capture + encode to the headless client.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release -p dcl-stream-server --features ffmpeg,uinput
cargo build --release -p dcl-stream-client --features player
cargo test --release --workspace --all-features

work=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$work"' EXIT

# A screen with something moving on it, and a sound server.
Xvfb :99 -screen 0 1280x720x24 -nolisten tcp &
sleep 1
DISPLAY=:99 xterm -geometry 120x40 -e 'while true; do date; ls -la /usr/bin | head -50; sleep 0.05; done' &
pulseaudio -D --exit-idle-time=-1 --system=false 2>/dev/null || true
sleep 1

token=$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
echo -n "$token" > "$work/token"
DISPLAY=:99 RUST_LOG=debug RUST_BACKTRACE=1 target/release/dcl-stream-server \
  --listen 127.0.0.1:47100 --token-file "$work/token" --fingerprint-file "$work/fp" \
  --encoders nvenc,vaapi,software --uinput false --audio default &
server_pid=$!
for _ in $(seq 50); do [[ -s "$work/fp" ]] && break; sleep 0.2; done
[[ -s "$work/fp" ]] || { echo "server did not start"; exit 1; }

target/release/dcl-stream-client --headless --seconds 6 \
  --addr 127.0.0.1:47100 --fingerprint "$(cat "$work/fp")" --token "$token" \
  --width 1280 --height 720 --fps 60 --codecs h264 > "$work/stats.json"
cat "$work/stats.json"
if ! kill -0 "$server_pid" 2>/dev/null; then
  status=0; wait "$server_pid" || status=$?
  echo "server exited during the stream (status $status)"; exit 1
fi
python3 - "$work/stats.json" <<'PY'
import json, sys
s = json.load(open(sys.argv[1]))["stats"]
assert s["frames_shown"] >= 150, f"too few frames: {s}"  # software x264 on a CI runner
assert s["frames_lost"] == 0, s
print(f"OK: {s['frames_shown']} frames, {s['audio_packets']} audio packets, "
      f"{s['bytes_video'] * 8 / 6 / 1e6:.1f} Mbit/s, latency {s['last_latency_us']} us")
PY
