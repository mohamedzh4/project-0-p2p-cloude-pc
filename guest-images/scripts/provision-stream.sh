#!/usr/bin/env bash
# Pro guest: build and install dcl-stream-server (our streamer, stream/ in
# this repo) inside the image, so it links the image's own FFmpeg. The Rust
# toolchain and -dev packages are removed again afterwards.
set -euo pipefail

if [[ "${LAB_TIER:-}" != "pro" ]]; then
  echo "provision-stream.sh is Pro-only" >&2
  exit 1
fi
SRC=/tmp/dcl-stream
[[ -f "$SRC/Cargo.toml" ]] || { echo "stream source missing at $SRC" >&2; exit 1; }
: "${RUST_TOOLCHAIN:?RUST_TOOLCHAIN is required (versions.env)}"

export DEBIAN_FRONTEND=noninteractive
apt-get update
build_deps=(build-essential pkg-config clang libclang-dev
  libavcodec-dev libavformat-dev libavdevice-dev libavutil-dev libavfilter-dev
  libswscale-dev libswresample-dev)
apt-get install -y --no-install-recommends "${build_deps[@]}" \
  libavcodec60 libavformat60 libavdevice60 libswscale7 libswresample4 curl ca-certificates

export RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$RUST_TOOLCHAIN" --no-modify-path
export PATH=/opt/cargo/bin:$PATH
(cd "$SRC" && cargo build --release --locked -p dcl-stream-server --features ffmpeg,uinput)
install -m 0755 "$SRC/target/release/dcl-stream-server" /usr/local/bin/dcl-stream-server

# Input: the streamer runs as ubuntu and creates its keyboard, mouse and
# gamepads through /dev/uinput.
echo uinput > /etc/modules-load.d/dcl-uinput.conf
cat > /etc/udev/rules.d/60-dcl-uinput.rules <<'RULES'
KERNEL=="uinput", SUBSYSTEM=="misc", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"
RULES
usermod -aG input ubuntu

install -m 0755 /tmp/dcl-stream-wait /usr/local/libexec/dcl-stream-wait
install -m 0644 /tmp/dcl-stream.service /etc/systemd/system/dcl-stream.service
systemctl enable dcl-stream.service

# Remove the build toolchain: only the binary and runtime libraries stay.
rm -rf /opt/rustup /opt/cargo "$SRC"
apt-get purge -y "${build_deps[@]}"
apt-get autoremove -y
apt-get clean
rm -rf /var/lib/apt/lists/*
