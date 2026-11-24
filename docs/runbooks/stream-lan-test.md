# Runbook: Pro gaming over the LAN with DCL Stream

Goal: play a game in the Pro guest on your Linux gaming PC, from another PC
on the same network, through our own streamer (`stream/`). No Matchmaker,
mesh or Host Agent is involved: this tests the image, the GPU passthrough
and the stream alone.

You need:

- **Gaming PC (host):** Linux with KVM, IOMMU on, and the GPU you give away
  bound to `vfio-pci`, plus a second display path (iGPU or second card) so
  the host keeps a screen. `host-agent/scripts/pro-gpu-check.sh` checks all of it.
- **Player PC:** Windows, Linux or macOS, on the same LAN.

## 1. Build the Pro image (gaming PC)

```bash
cd guest-images
openssl genpkey -algorithm ED25519 -out ~/dcl-dev-signing.pem   # once; never commit it
IMAGE_SIGNING_PRIVATE_KEY=~/dcl-dev-signing.pem PACKER_ACCELERATOR=kvm ./scripts/build.sh pro
```

Needs `packer`, `qemu-img`, `openssl` and about 30 GB free. It takes 30-90
minutes: it installs the NVIDIA driver and Mesa, Steam with Proton, Wine and
Lutris, and compiles `dcl-stream-server` inside the image. Building it accepts
the Steam installer's licence.

Output: `guest-images/output/pro/rootfs.img`.

## 2. Bind the GPU to vfio-pci (gaming PC)

Find the GPU and its HDMI audio function:

```bash
lspci -nn | grep -Ei 'vga|3d|audio'
```

Bind both (example addresses; use yours), with the desktop on the other GPU:

```bash
sudo modprobe vfio-pci
for d in 0000:01:00.0 0000:01:00.1; do
  echo vfio-pci | sudo tee /sys/bus/pci/devices/$d/driver_override
  echo $d | sudo tee /sys/bus/pci/devices/$d/driver/unbind 2>/dev/null
  echo $d | sudo tee /sys/bus/pci/drivers_probe
done
```

To give it back afterwards: write an empty line to `driver_override`, unbind from
`vfio-pci`, and probe again (or reboot).

## 3. Boot the guest (gaming PC)

```bash
./scripts/run-pro-lan-test.sh output/pro/rootfs.img 0000:01:00.0 0000:01:00.1
```

It prints the exact client command, including a fresh session token. Changes
inside the guest (installed games) are thrown away on exit unless you start it
with `DCL_KEEP_CHANGES=1`.

Allow UDP 47100 in the gaming PC's firewall if it has one.

## 4. Build and run the client (player PC)

- **Windows:** download `dcl-stream-client-windows` from the latest
  "Stream" workflow run in GitHub Actions (exe + FFmpeg DLLs), or build it:
  install Rust and LLVM, unpack an FFmpeg 7.1 *shared* build, set
  `FFMPEG_DIR` to it, then `cargo build --release -p dcl-stream-client --features player`
  in `stream/`.
- **Linux:** install the FFmpeg dev packages (`libavcodec-dev libavformat-dev
  libavutil-dev libswscale-dev libswresample-dev`), `clang`, `libasound2-dev`,
  `libudev-dev`, then the same `cargo build`.
- **macOS:** `brew install ffmpeg@7 pkg-config`,
  `export PKG_CONFIG_PATH="$(brew --prefix ffmpeg@7)/lib/pkgconfig"`, then build.

Run the command the guest script printed:

```bash
dcl-stream-client --addr 192.168.1.20:47100 --token <64 hex>
```

Options: `--width 2560 --height 1440 --fps 120`, `--codecs hevc,h264`,
`--max-bitrate-kbps 60000`, `--headless --seconds 30` (no window; prints stats).

In the window: click to capture the mouse; **Ctrl+Alt+Shift+Z** releases it,
**Ctrl+Alt+Shift+F** fullscreen, **Ctrl+Alt+Shift+Q** quit. The title bar
shows fps, latency, bitrate, FEC and lost frames.

## 5. Play

Steam is in the menu. For Windows games: Steam → Settings → Compatibility →
"Enable Steam Play for all other titles" (Proton). Non-Steam Windows games:
Lutris or `wine setup.exe`.

## What to record

| Check | How |
|---|---|
| GPU works in the guest | `nvidia-smi` (NVIDIA) or `vulkaninfo --summary` in a guest terminal |
| Encoder in use | first line of the client output (`h264_nvenc`, `hevc_vaapi`, `libx264`...) |
| Frame rate and latency | client title bar, or `--headless --seconds 30` JSON |
| Loss behaviour | same JSON: `frames_lost`, `frames_recovered_by_fec`, `fec_pct` |
| Stock Sunshine on the same guest | Moonlight to the guest's address (Sunshine is still in the image) |

## When it does not work

- **Black picture, frames counting:** X runs on the wrong GPU or with no
  screen. In the guest: `journalctl -b -u dcl-display-setup` and
  `/etc/X11/xorg.conf.d/20-dcl-gpu.conf`. An HDMI dummy plug on the GPU always works.
- **Client cannot connect:** `journalctl -b -u dcl-stream` in the guest
  (the serial log shows boot progress). The service waits up to 10 minutes for
  the token, X and the sound server.
- **No encoder opened:** the NVIDIA driver did not load (`lsmod | grep nvidia`),
  or VAAPI is missing (`vainfo`). The server falls back to x264 when it can.
- **Slow or choppy over QEMU user networking:** this test forwards UDP through
  QEMU's SLIRP, which is slow for UDP (design section 3). Note the numbers; a
  TAP bridge or passt is the fix.

## Known limits of this version

- NVIDIA recovers lost frames with a (rate-limited) keyframe, not reference-frame
  invalidation: that needs the direct NVENC SDK path (design phase 6).
- Capture copies through X11 (XShm); zero-copy KMS capture is phase 6.
- Bitrate changes reopen the FFmpeg encoder (one keyframe each).
- No Matchmaker integration yet (phase 5): the token is passed by hand.
