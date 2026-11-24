# DCL Stream: our own game-streaming protocol (design)

Date: 2026-11-24. Status: **phases 1-4 built** in `stream/` (see section 10),
not yet run on a GPU. Replaces Sunshine and Moonlight for the Pro tier once it
matches them in measurements (phase 6). Analysis of their code:
`docs/sunshine-moonlight-analysis.md`. LAN test: `docs/runbooks/stream-lan-test.md`.

Labels as in `docs/rapport-protocoles-reseau-pipeline.md`: **Estimate** means
reasoned, not measured. Every speed number below is an estimate until the
latency harness (phase 2) measures it.

## 1. Why our own

Sunshine/Moonlight (NVIDIA GameStream) are good, but for this project:

- **Pairing is wrong for us.** A PIN typed by a person, a client certificate kept
  forever. We need one key per booking, issued by the Matchmaker, gone at teardown.
- **The admin web UI (47990) is an attack surface** we have to fence off in the
  ACL. Our server has no admin UI at all.
- **The protocol is undocumented** and GPL-3.0. We cannot change it without
  keeping a public fork, and we cannot tune it per network the way we want.
- **No browser client.** Moonlight needs an app. Our protocol is built on QUIC,
  so the same protocol can run in a browser over WebTransport (phase 7).

What we keep from GameStream, because it is why it is fast:

- UDP for video and audio, never TCP (no head-of-line blocking).
- Hardware encoding (NVENC, VAAPI, AMF) with low-latency settings.
- Reed-Solomon FEC per frame, so small losses are repaired without a resend.
- Loss reports from the client, and recovery with a reference-frame
  invalidation or a keyframe instead of a long freeze.
- A separate input channel that is never stuck behind video.

We write our own code from this design. **We do not copy Sunshine or Moonlight
source** (GPL-3.0); we only use the published ideas.

## 2. What we promise, and how we check it

"Guaranteed speed" cannot be promised: the renter's network decides the floor.
What we can promise and test:

| Promise | How it is checked |
|---|---|
| Security properties in section 6 hold | Unit tests, fuzzing of every parser, a test that a wrong token / wrong cert / second client is refused |
| Added latency (everything except the network) stays within the budget in section 5 | Per-frame timestamps in the protocol; the client shows and logs each stage |
| At equal network conditions we are no slower than stock Sunshine + Moonlight | A/B test on the same Pro host (phase 6). Sunshine stays in the image until we pass |
| Under 1–5 % loss the picture recovers without freezes longer than one keyframe | `tc netem` tests (report item S4) |

## 3. Where it runs

```
renter PC                         mesh (WireGuard, Headscale)                 host PC
┌───────────────────┐                                          ┌────────────────────────────┐
│ dcl-stream-client │ ── QUIC over UDP, inside WireGuard ───── │ Pro guest (Ubuntu 24.04)   │
│ (desktop app)     │    direct P2P when NAT allows,           │  dcl-stream-server         │
│ joins the mesh as │    DERP relay otherwise                  │  GPU via VFIO / DDA / GPU-P│
│ a per-session node│                                          └────────────────────────────┘
└───────────────────┘
browser (phase 7) ── WebTransport (QUIC) ── VPS relay ── mesh ── guest
```

- **Server** `dcl-stream-server`: a Rust binary in the Pro guest image, started
  by systemd after `dcl-guest-join`. Listens on the guest's mesh IP only.
- **Client** `dcl-stream-client`: a Rust desktop app (Windows, macOS, Linux).
  For a booking it gets from the Matchmaker: the guest's mesh address, the
  server's certificate fingerprint, a session token and a one-time mesh key.
- **Today the renter is not on the mesh** (report 4.5). Phase 5 adds an
  ephemeral `tag:renter` node per session, with an ACL that lets that node reach
  only its own guest's stream port.

Known path risk to measure early: the guest's WireGuard UDP goes through QEMU
user networking (SLIRP / passt) on Linux hosts and the Hyper-V switch on
Windows. SLIRP is slow for UDP; if it caps throughput we switch Pro guests to
passt or a TAP bridge.

## 4. Protocol

One QUIC connection per session (Rust `quinn`, TLS 1.3, ALPN `dcl-stream/1`).
QUIC gives us encryption, congestion control and multiplexing from a standard,
reviewed library, so we write no crypto of our own.

| Channel | QUIC type | Carries |
|---|---|---|
| Control | bidirectional stream 0, reliable | hello/auth, settings, keyframe requests, loss + RTT reports, clock sync, session end |
| Input (ordered) | unidirectional stream, reliable | key down/up, mouse buttons, gamepad buttons, text |
| Input (latest wins) | datagrams | mouse motion, gamepad axes, touch; sequence number, old ones dropped |
| Video | datagrams | encoded frames, split into shards, Reed-Solomon parity |
| Audio | datagrams | Opus packets, 5 ms or 10 ms, Opus in-band FEC |

### 4.1 Session start

1. Client opens QUIC to `guest-mesh-ip:47100/udp`. It accepts the server
   certificate only if its SHA-256 fingerprint matches the one it expects.
   The server's Ed25519 key is derived from the session token
   (`auth::server_key_seed`), and the certificate is deterministic, so the
   client computes the expected fingerprint from the token itself. The guest
   never reports a fingerprint back.
2. Client sends `Open{version}`. Server answers `Hello{version, nonce,
   capabilities}` (codecs, max resolution, encoders found).
3. Client answers `Auth{HMAC-SHA256(session_token, nonce ‖ tls_exporter)}`. The
   TLS exporter binds the proof to this connection, so it cannot be replayed or
   relayed. Wrong proof → connection closed, nothing else sent.
4. Client sends `Settings{width, height, fps, codec, max_bitrate, hdr=false}`.
   Server picks the encoder and starts with a keyframe.

### 4.2 Video and audio datagram header (24 bytes, little-endian)

| Field | Bytes | Meaning |
|---|---|---|
| kind | 1 | 1 video, 2 audio (3 = input datagram, no shard header) |
| flags | 1 | KEYFRAME, RECOVERY (first frame after reference-frame invalidation) |
| frame_id | 4 | increases by one per frame, per kind |
| group, group_count | 1 + 1 | FEC group of this shard, groups in the frame (max 16) |
| shard | 2 | 0..data+parity-1 |
| data_shards, parity_shards | 2 + 2 | k and m of this group (max 1024 each) |
| group_len | 4 | real bytes in the group (last data shard is zero-padded) |
| capture_us | 6 | server clock, microseconds, when the frame was captured |

Packets are a fixed 1200 bytes (QUIC MTU discovery off: the mesh MTU is 1280
and a lost probe would shrink packets under shards already cut).

Shard size is fixed per session so it fits one datagram without fragmentation.
The mesh MTU is 1280, so a shard is about 1100 bytes (estimate, after IPv6 +
UDP + QUIC + header). Frames bigger than 255 shards are split into slices, each
with its own FEC group.

### 4.3 Loss, recovery and rate control

- Client reassembles a frame when any k of k+m shards arrive. If it cannot, it
  reports `FrameLost{frame_id}` on the control stream at once.
- Server answers with reference-frame invalidation when the encoder supports it
  (NVENC), otherwise a keyframe. Keyframes are rate-limited (at most 1 per
  200 ms) so a bad network does not turn into a keyframe storm.
- **Adaptive FEC:** parity starts at 15 % and follows measured loss (5–40 %).
- **Bitrate:** the server watches delay growth between capture and arrival
  (reported by the client) and loss, lowers the encoder bitrate before queues
  build up, and raises it slowly. QUIC's own congestion window is the hard cap.
- **Frame pacing:** shards of one frame are spread over a few ms, not sent in
  one burst, to avoid router queue drops.

### 4.4 Clock sync and telemetry

The control stream carries ping/pong every second; the client estimates the
server clock offset (NTP-style). Each frame then has: capture, encode done,
sent, last shard received, decoded, presented. The client shows these live (an
overlay, like Moonlight's stats) and writes them to a file; the Host Agent's
quality score can use the summary later.

## 5. Pipeline and latency budget (estimates, 1080p60)

| Stage | Guest / client | Plan | Budget |
|---|---|---|---|
| Capture | guest | phase 2: XShm (CPU copy). Phase 6: KMS/DRM DMA-BUF, zero copy | 1–3 ms |
| Encode | guest | FFmpeg libavcodec: `h264_nvenc` / `hevc_nvenc` / `av1_nvenc`, VAAPI, AMF; x264 `ultrafast` as last fallback. No B-frames, infinite GOP + intra refresh, CBR, VBV ≈ 1 frame | 3–8 ms |
| Packetize + FEC | guest | `reed-solomon-simd` | < 1 ms |
| Network | both | WireGuard + QUIC | RTT / 2 (not ours) |
| Jitter wait | client | none by default; waits only for the last shard | 0–5 ms |
| Decode | client | FFmpeg hardware decode (D3D11VA, VideoToolbox, VAAPI) | 2–5 ms |
| Present | client | `wgpu`, mailbox / immediate present, no extra buffering | 0–16 ms |
| **Added total** | | target, to be measured | **≤ 20 ms + network** |

Audio: PipeWire capture of the default sink monitor, Opus 48 kHz stereo,
5–10 ms frames, client keeps a 20 ms buffer. Input: `uinput` virtual keyboard,
relative mouse, absolute tablet and Xbox-layout gamepads in the guest; the
client uses `winit` and `gilrs`.

The guest desktop stays XFCE on X11 (`provision-sunshine.sh`), so phase 2 can
capture with XShm without changing the image.

## 6. Security model

| Threat | Defence |
|---|---|
| Someone else on the mesh connects to the stream | ACL: only the session's renter node (or the relay) may reach UDP 47100 on that guest. Then the token proof in 4.1 |
| Stolen or replayed token | Token is per booking, 32 random bytes, hash given to the guest in its bootstrap, never sent in clear; the proof is bound to the TLS connection; token dies at teardown |
| Fake server (MITM) | Client pins the server certificate fingerprint from the Matchmaker. The guest makes its key at boot and reports only the fingerprint to the Matchmaker over its bootstrap channel |
| Second client steals the session | One authenticated client at a time. A reconnect with a valid proof replaces the old one (renter's network changed); anything else is refused |
| Malformed packets crash or own the server | Parsers in a no-I/O crate with no `unsafe`, hard size limits, `cargo-fuzz` targets in CI |
| Host reads the stream | It sees only WireGuard UDP. It can still read guest memory (report section 6); same as today |
| Admin UI abuse (Sunshine 47990) | There is no admin UI. Settings come only from the authenticated client, within limits the Matchmaker set for the tier |
| Input abuse | Input only reaches the renter's own guest; key codes and axis ranges are validated; no clipboard or file transfer in version 1 |

Encryption is doubled (WireGuard outside, QUIC TLS inside). The cost is small
(AES-GCM at a few GB/s against a 20–80 Mbit/s stream, estimate), and the inner
layer is what protects the browser path in phase 7, where there is no WireGuard
between the browser and the relay.

## 7. Code layout

```
stream/                      new Cargo workspace
  proto/    dcl-stream-proto   message types, header, shard/FEC, rate control, no I/O
  server/   dcl-stream-server  guest binary: capture, encode, audio, uinput, quinn
  client/   dcl-stream-client  desktop: quinn, decode, wgpu, cpal, winit, gilrs
  fuzz/                       cargo-fuzz targets for proto
guest-images/scripts/provision-stream.sh   installs the server (Pro)
```

Matchmaker additions (phase 5): stream token and fingerprint fields on the
session, an endpoint for the client to fetch them, renter mesh key issue.
Host Agent: health probe on 47100/udp instead of Sunshine 47989; locator
returns the stream address instead of the Sunshine web UI.

## 8. Phases

Each phase ends with something we can run and a pass/fail check.

| # | Builds | Done when |
|---|---|---|
| 1 | `proto` crate: header, shards, Reed-Solomon, loss simulation, fuzz targets | Tests pass on the Mac; frames survive 10 % simulated random loss with 25 % parity |
| 2 | Server MVP (XShm + FFmpeg encode + quinn) and client MVP (decode + display), per-stage telemetry | The Arch PC streams its guest desktop to another PC on the LAN; latency numbers are logged |
| 3 | Input (uinput) and audio (PipeWire + Opus) | Play a game for 30 min on LAN |
| 4 | Recovery, adaptive FEC, bitrate control | `netem` 1/2/5 % loss and 50 ms jitter: no freeze over 200 ms |
| 5 | Matchmaker token + fingerprint, guest bootstrap, renter mesh node, ACL, locator, Host Agent probe | A real booking from the website starts a stream; wrong token and second client refused |
| 6 | KMS capture, zero-copy encode, A/B against Sunshine | Same or lower latency than Sunshine on the same host; then Sunshine leaves the image |
| 7 | Browser client: WebTransport + WebCodecs + WebAudio through a VPS relay | Renter plays from a browser with no install |

## 9. Open decisions

1. Desktop client first (fastest path, direct P2P), or browser first (no
   install, but every packet goes through the VPS)? This design picks desktop first.
2. Keep Sunshine in the Pro image as a fallback until phase 6? This design says yes.
3. UDP port 47100 is a placeholder; any free port works.

## 10. What is built (2026-11-24)

| Phase | State |
|---|---|
| 1 proto | Done: `stream/proto`, 37 unit tests (header, FEC, reassembly with speculative loss, rate control, pacing, clock, auth, QUIC config) |
| 2 server + client MVP | Done: `stream/server` (x11grab + NVENC/VAAPI/x264 through FFmpeg, synthetic backend), `stream/client` (headless + `player` window). Loopback tests over real QUIC; Linux check streams a real X screen with x264 (`stream/scripts/linux-check.sh`) |
| 3 input + audio | Done in code: uinput keyboard/mouse/pointer/gamepads, PulseAudio + Opus 5 ms; client winit/cpal/gilrs. Not yet tried with a real game |
| 4 recovery + adaptation | Done: loss reports every 50 ms, speculative loss, RFI or rate-limited keyframe, adaptive FEC and bitrate. Tested with a lossy proxy (5 % random, 30 % bursts) |
| 5 Matchmaker integration | Not started. Guest side ready: `dcl-guest-join` stores `streamToken` from the bootstrap, or `dcl.stream_token` for LAN tests |
| 6 performance | Not started (KMS zero-copy, NVENC SDK for RFI, A/B vs Sunshine) |
| 7 browser | Not started |

Gaming image (Pro): `guest-images/scripts/provision-gaming.sh` (drivers,
Steam + Proton, Wine, Lutris, virtual monitor and sound card),
`provision-stream.sh` (builds the server in the image), `run-pro-lan-test.sh`.
