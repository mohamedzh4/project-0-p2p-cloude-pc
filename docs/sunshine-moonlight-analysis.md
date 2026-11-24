# Sunshine and Moonlight: what they do, and what we take

Date: 2026-11-24. Read (not built, not run):

| Repo | Commit | Licence |
|---|---|---|
| LizardByte/Sunshine (server) | 433c876, 2026-11-23 | GPL-3.0 |
| moonlight-stream/moonlight-common-c (protocol core of every client) | b4ee1ce, 2026-11-22 | GPL-3.0 |
| moonlight-stream/moonlight-qt (desktop client) | d0ce158, 2026-11-23 | GPL-3.0 |

Goes with `docs/streaming-protocol-design.md`. Section 4 lists the changes this
analysis makes to that design.

**Licence rule we follow:** we copy no code. We take ideas, protocol
mechanisms and encoder parameter values (facts about how NVENC, VAAPI and AMF
behave), and write our own Rust. If we ever copy a GPL file, the whole program
that contains it must become GPL.

## 1. How they work, part by part

### 1.1 Session setup

- **Sunshine:** HTTP API on 47989 and HTTPS with client certificates on 47984 (`nvhttp.cpp`), PIN pairing, then RTSP on 48010 (`rtsp.cpp`). The client sends its settings as SDP lines like `x-nv-vqos[0].fec.minRequiredFecPackets`.
- **Moonlight:** `RtspConnection.c` and `SdpGenerator.c`.
- **Verdict: skip all of it.** It is three protocols (HTTP, RTSP, SDP) and a
  pairing ceremony that exist because NVIDIA built it in layers. One QUIC
  handshake plus our token proof (design 4.1) does the same job.

### 1.2 Video packets

- **Sunshine:** `videoBroadcastThread`, `stream.cpp:1610`.
  - Each shard is an RTP header, 4 reserved bytes and `NV_VIDEO_PACKET` (frame index, packet index, FEC info, flags SOF/EOF).
  - Before the payload there is an 8-byte frame header: frame type (P / IDR / intra-refresh / after-RFI), the last packet's real length, and encode latency in 0.1 ms.
  - With encryption on, each shard gets AES-GCM with a counter IV (12 bytes IV, 16 bytes tag, 4 bytes frame number).
- **Take:** the frame types, sending real length of the last shard (needed for
  AV1), and **putting the server's processing latency in every frame**: that
  is how Moonlight shows "host processing latency". Our header carries a
  capture timestamp instead, which gives the same and more.
- **Skip:** the RTP header (we are not RTP), our own AES-GCM (QUIC encrypts).

### 1.3 Forward error correction

- **Sunshine** (`stream.cpp:800`, `1685`):
  - Uses Reed-Solomon over GF(2^8) (the `nanors` library), so at most 255 shards per block. A frame can be split into **at most 4 FEC blocks**. Frames bigger than that go out with **FEC turned off** (log line "Skipping FEC for abnormally large encoded frame").
  - Parity is a **fixed percentage** from the config, 20 % by default (`config.cpp:819`).
- **Moonlight** (`RtpVideoQueue.c`): rebuilds a block as soon as any k of k+m shards are in.
- **Audio** uses a fixed 4 data + 2 parity per block (`RtpAudioQueue.h`).
- **Take:** the per-frame Reed-Solomon idea and audio's 4+2.
- **Do better:**
  - Use `reed-solomon-simd` (GF(2^16), up to 65535 shards, SIMD-fast). No 4-block limit, so keyframes never lose their FEC. Keyframes are exactly the frames you most need to arrive.
  - Make parity follow the measured loss (1.6).

### 1.4 Pacing and send path

- **Sunshine** (`stream.cpp:1735`):
  - Paces at a **fixed 80 % of 1 Gbit/s**, in 1 ms groups.
  - Sends with batched system calls (`sendmmsg` / GSO on Linux), at most 64 packets or 64 KB per batch. Larger batches misbehave on Windows.
- **Take:** batching and GSO. `quinn` already does GSO/GRO on Linux, so we get this for free.
- **Do better:** pace at the **measured** path rate, not a 1 Gbit/s
  assumption. Over the internet and through WireGuard, 1 Gbit/s bursts
  overflow home routers, and that's a common cause of loss in streaming.

### 1.5 Encoder settings

From `video.cpp`. These are the settings that make low latency, and we reuse
the values:

| Encoder | Settings |
|---|---|
| NVENC | `tune` = ultra-low-latency, `rc` = CBR, `zerolatency` = 1, `delay` = 0, `forced-idr` = 1, no CBR padding, preset from config |
| Quick Sync | `async_depth` = 1, `low_power` = 1 (fallback 0 on old GPUs) |
| AMF | `usage` = ultra-low-latency, `async_depth` = 1, no filler data, `enforce_hrd` |
| VAAPI | low-latency RC, limited GOP |
| All | **VBV buffer = one frame** (`rc_buffer_size = bitrate / fps`, `video.cpp:2527`), so no frame can be much bigger than average; no B-frames |

Encoder search order on Linux: NVENC, then Vulkan encode (optional build), then VAAPI, then software (`video.cpp:1605`).

- **Bitrate is fixed for the whole session** (`video.cpp:2501`): the client
  picks it at the start and it never changes.
- **Do better:** change bitrate during the session (1.6). NVENC can reconfigure
  bitrate without a keyframe (`nvEncReconfigureEncoder`). Whether FFmpeg's
  VAAPI path can is **to be checked**. If not, we step bitrate only at a keyframe.

### 1.6 Loss handling and recovery

- **Moonlight** (`ControlStream.c`):
  - Sends a **loss report every 50 ms** (`LOSS_REPORT_INTERVAL_MS`).
  - Asks for a **reference-frame invalidation (RFI)** when a frame is lost, or an IDR when RFI can't work.
  - **Speculative RFI** (`RtpVideoQueue.c`): when packets arrive in order and enough shards of a frame are already missing that it can't be rebuilt, the client asks for RFI *before the frame ends*. It turns this off for 5 minutes if it sees reordering or guessed wrong.
- **Sunshine:**
  - Does RFI **only with its own NVENC code on Windows** (`nvenc/nvenc_base.cpp:799`, flag `REF_FRAMES_INVALIDATION` on the Windows encoder only). On Linux it uses FFmpeg's NVENC, which has **no RFI**, so every unrecoverable loss costs a full keyframe.
  - **It ignores the loss reports**: `IDX_LOSS_STATS` is only written to the log (`stream.cpp:1205`). Nothing adapts.
- **Moonlight** also rates the connection "poor" or "okay" from the frame-loss percentage per interval (`ControlStream.c:522`). It only shows this to the user.
- **Take:**
  - Loss reports every 50 ms.
  - RFI with an IDR fallback.
  - Speculative RFI, including its safety switch.
- **Do better:**
  - **Act on the reports.** Loss and delay drive FEC percentage and encoder bitrate. This is the biggest gap in Sunshine.
  - **Use the NVENC SDK directly on Linux** (through CUDA), not FFmpeg, so our Linux guest gets RFI. Our Pro guests are Linux, so in Sunshine they'd always be on the no-RFI path.
  - Rate-limit keyframes (design 4.3).

### 1.7 Capture (Linux guest)

- **Sunshine** (`platform/linux/`):
  - `x11grab.cpp`: XShm copy to RAM, plus the XFixes cursor drawn in.
  - `kmsgrab.cpp`: reads the scanout buffer straight from DRM. It needs `CAP_SYS_ADMIN`, which it holds only while opening (`privileged_drm_worker`). It exports the buffer as a DMA-BUF (`drmPrimeHandleToFD`), imports it into EGL/GL, converts RGB to NV12 on the GPU, then passes it to CUDA (`cuda.cpp`, `cuGraphicsGLRegisterImage`) or VAAPI. **Zero copy.**
  - `cuda.cpp`: NvFBC for NVIDIA.
  - `wlgrab.cpp`, `portalgrab.cpp`, `pipewire.cpp`: Wayland.
- **Take:** XShm first (simple, works on our XFCE/X11 guest), then KMS zero
  copy with the same privilege trick. In KMS the cursor is a separate plane,
  so it has to be drawn in ourselves.
- **Not mentioned in their code but critical for us:**
  - A GPU passed to a VM usually has **no screen attached**, and with no display there's nothing to capture. Sunshine assumes a real monitor or a dummy HDMI plug.
  - Our guest image must give the NVIDIA X server a virtual display: `AllowEmptyInitialConfiguration`, plus a `ConnectedMonitor` or `CustomEDID` in `xorg.conf`. AMD needs an EDID override on the kernel command line.

### 1.8 Audio

- **Sunshine** (`platform/linux/audio.cpp`, `audio.cpp`):
  - Creates a PulseAudio **null sink** and records its monitor.
  - Opus multistream with `OPUS_APPLICATION_RESTRICTED_LOWDELAY`, **CBR** (`VBR(0)`), 5 ms packets at 48 kHz.
- **Take all of it.** It works on PipeWire too, through `pipewire-pulse`.

### 1.9 Input

- **Sunshine:** `platform/linux/input/virtualhid.cpp`, virtual HID devices through uinput (keyboard, mouse, touch, pen, gamepads). It also has rumble, motion and LED messages back to the client (`stream.cpp:55`, types 0x55xx).
- **Moonlight** (`InputStream.c`):
  - Queues up to 150 input packets.
  - **Batches mouse motion**: it adds up relative moves that arrive within about a millisecond into one packet. Its comment says this lowers latency, because it stops a queue building up.
  - Never batches button or key events.
- **Take:**
  - uinput devices.
  - Motion batching, with keys and buttons sent on their own.
  - Rumble back to the client.
- **Skip in version 1:** pen, motion sensors, RGB LEDs, adaptive triggers.

### 1.10 Client decode and display

- **moonlight-qt** (`app/streaming/video/ffmpeg-renderers/`):
  - Decodes with FFmpeg hardware acceleration per OS: D3D11VA and DXVA2 (Windows), VAAPI, VDPAU and CUDA (Linux), VideoToolbox (macOS), DRM.
  - Renders with zero-copy where it can (EGL images, D3D11 textures, Metal).
  - **Pacer** (`pacer/pacer.cpp`):
    - A vsync thread and a render queue.
    - When frames pile up, it drops old ones until the queue holds 1 (up to 3 if the history shows jitter), so latency never grows.
- **Take:**
  - FFmpeg hardware decode per OS.
  - The drop-to-newest pacer.
  - An option to show frames as soon as they're decoded, accepting tearing, as the lowest-latency mode.
- **Hard part for us:** handing a hardware-decoded frame to a Rust renderer
  without a copy. The plan:
  1. First version: copy the decoded frame to the CPU and upload it as a texture. This adds roughly 1–3 ms at 1080p (estimate).
  2. Later: zero-copy per OS, D3D11 shared textures on Windows and DMA-BUF/EGL on Linux.

### 1.11 Security

- **Sunshine:**
  - Pairing uses the PIN plus certificates.
  - The control stream is AES-GCM encrypted.
  - Video encryption is optional, with counter IVs per NIST SP 800-38D (`stream.cpp:1810`).
  - It has a web UI with admin rights, which our ACL already has to fence off.
- **Verdict:** QUIC TLS 1.3 encrypts everything, video included. Session
  tokens replace pairing, and there's no web UI (design section 6).

### 1.12 Things we skip entirely

mDNS discovery, UPnP port mapping (`upnp.cpp`), the web config UI
(`confighttp.cpp`), app list and game launching (`process.cpp`), several
clients at once, HDR (later), tray icon, display mode switching on Windows
(`display_device.cpp`). That is roughly half of Sunshine's code.

## 2. Score: where Sunshine/Moonlight are strong and weak for us

| Area | Them | Us (planned) |
|---|---|---|
| Transport | Raw UDP, fast, no congestion control | QUIC datagrams, same speed path, encrypted always. **Risk:** QUIC congestion control must not throttle video (section 3) |
| FEC | Fixed 20 %, off for very large frames | Adaptive, never off |
| Bitrate | Fixed per session | Follows the network |
| Loss reports | Sent by client, **ignored** by server | Drive FEC and bitrate |
| RFI on a Linux guest | No (FFmpeg NVENC) | Yes (NVENC SDK) |
| Pacing | Fixed 800 Mbit/s bursts | Measured path rate |
| Setup | HTTP + RTSP + SDP + PIN | One QUIC handshake + token |
| Encoders, capture, audio, input | Mature, many GPUs | We reuse their settings; less tested than theirs at first |
| Clients | Every OS, TVs, phones | Windows, Linux, macOS desktop first |

## 3. Risks found while reading

1. **QUIC congestion control and real-time video.**
   - The default controllers (NewReno, Cubic) halve their send window on loss. With datagrams, that means quinn drops our packets locally.
   - Fix: run quinn with BBR (it has one), or with a window large enough that our own rate controller is the real limit.
   - Has to be measured in phase 2.
2. **Mesh MTU 1280.** Moonlight's default packet size (1392) would fragment inside WireGuard. QUIC packets of 1200–1232 bytes fit. Shard payload is about 1100 bytes (estimate), so a 1080p keyframe of about 200 KB is around 180 shards.
3. **No screen on a passthrough GPU** (1.7). Without the virtual display config, capture gets nothing.
4. **Zero-copy decode in Rust** (1.10) is real work per OS.

## 4. Changes to `streaming-protocol-design.md`

1. FEC: `reed-solomon-simd` (GF(2^16)); no 4-block limit; keyframes keep FEC.
2. Recovery: speculative RFI on the client; NVENC SDK directly on Linux for
   RFI; FFmpeg encoders (VAAPI, AMF, software) fall back to keyframes.
3. Loss report every 50 ms, which feeds FEC and bitrate.
4. Pacing at the measured rate; GSO through quinn.
5. Encoder parameter table from 1.5 becomes our defaults.
6. Audio: Opus RESTRICTED_LOWDELAY, CBR, 5 ms, FEC 4+2 on top.
7. Input: motion batching; rumble back to the client.
8. Client pacer: drop to newest, queue at most 1.
9. Guest image: virtual display for NVIDIA and AMD (no monitor on the VM's GPU).
10. quinn with BBR or a large window, and a phase 2 test that proves congestion control isn't throttling video.

## 5. What needs your approval

- **A. The list in section 4** as changes to our design.
- **B. Desktop client first** (Windows + Linux + macOS), browser later.
- **C. The server is Linux only** (our Pro guest is Linux). "Works on Windows"
  means: the client runs on Windows, and Windows hosts serve it through the
  Hyper-V Linux guest. A Windows-guest server (DXGI capture, Windows NVENC)
  would be a later, separate piece.
- **D. Keep Sunshine in the image** as a fallback until our version passes the
  A/B test.
