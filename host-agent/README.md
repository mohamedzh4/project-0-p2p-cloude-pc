# Host Agent

The program that runs on a volunteer's PC ("Host") in the Distributed Compute Lab.

This is the Host Agent after execution phase 14 (Headscale mesh join). It
does these things and nothing else:

1. Detects the machine's CPU, RAM and GPU.
2. Lets the host cap how much of each they are willing to share.
3. Saves that choice to a local config file (v2).
4. Registers those **clamped** caps with the Matchmaker from **Rust**
   (`POST /v1/hosts` or `PUT /v1/hosts/{id}`). New hosts stay `pending` until an
   operator approves them with `HOST_APPROVE_TOKEN`. This agent never sends that
   token. When `HOST_AGENT_LISTEN_ADDR` is set, registration includes `agentUrl`.
5. While local `sharingEnabled` is on and a host id exists, pings
   `POST /v1/hosts/{id}/heartbeat` from Rust every 10 seconds. Pause sharing to
   stop pings immediately (next tick) so the host goes stale in matching.
6. Selects a **stub** provision target from host OS × lab tier (Linux Standard
   → Firecracker, Windows Standard → QEMU+WHPX, Pro → QEMU+VFIO / Hyper-V GPU).
   macOS is an explicit unsupported variant. No hypervisor is invoked.
7. Optionally listens for `POST /v1/provision` from the Matchmaker
   (`HOST_AGENT_LISTEN_ADDR`). The handler verifies the advertised SHA-256
   digest + Ed25519 signature against `IMAGE_SIGNING_PUBLIC_KEY` and
   `IMAGE_LOCAL_DIGEST`, then calls `plan_provision` and returns
   `https://provision.invalid/{backend}`. Mismatch is 400 (no stub boot).
   The webview does not `fetch` this.
8. Shows local presence: paused / idle / in-session. Earnings are
   `{n} credits accrued (not paid out)` from `GET /v1/hosts/{id}/earnings`
   (Rust, not the webview). Zero if this machine has no host id yet.
9. If `HEADSCALE_LOGIN_SERVER` is set, after register it POSTs
   `/v1/hosts/{id}/mesh/preauth` from Rust and runs `tailscale up` with that
   login-server (env) and the minted key. Missing mesh/Matchmaker env fails
   fast. It does **not** pass `--advertise-exit-node`. Omit the login-server
   env to skip join (UI-only).
10. Sits in the system tray (Rust, not the webview): tooltip, pause/resume,
    open window, quit. Closing the window hides to the tray when the icon
    installed.

There is no real Firecracker or QEMU. See
`../docs/distributed-compute-lab-architecture.md` for where those land.

## Stack

- **Rust** core (`src-tauri/`) — all system access and Matchmaker HTTP live here.
- **Tauri v2** shell — a native webview instead of a bundled browser, so the
  agent stays small enough to sit on someone's daily-driver machine.
- **React + TypeScript + Vite** for the window contents.

`sysinfo` is asked for the narrowest refresh it supports (no process list, no
disks, no networks). NVIDIA GPUs are read through `nvml-wrapper`; anything else
reports "No GPU detected" with a note explaining why.

The Content-Security-Policy allows no remote origins. Matchmaker calls belong
in Rust, not `fetch` from the webview.

## Layout

```
host-agent/
├── .env.example              MATCHMAKER_URL + HOST_BOOTSTRAP_TOKEN
├── src/                      UI
│   ├── App.tsx               window contents and state
│   ├── api.ts                the only place that calls Tauri commands
│   ├── components/           HardwareCard, ResourceSlider, GpuShareField
│   ├── format.ts             byte and core formatting
│   ├── preview.ts            sample data for browser-only development
│   └── types.ts              mirrors the Rust models
└── src-tauri/
    ├── src/
    │   ├── models.rs         shared types, recommended defaults, clamping
    │   ├── hardware.rs       CPU + RAM detection
    │   ├── gpu.rs            NVIDIA detection with graceful fallback
    │   ├── config.rs         load/save the sharing config; immediate pause
    │   ├── identity.rs       persisted Matchmaker host id
    │   ├── matchmaker.rs     inventory + heartbeat + earnings GET (clamp, fail-fast env)
    │   ├── mesh.rs           Matchmaker preauth mint + tailscale up from env (no secrets in source)
    │   ├── agent_status.rs   paused / idle / in-session (no Tauri types)
    │   ├── provision.rs      OS-conditional stub backends (peers; no hypervisor)
    │   ├── image.rs          SHA-256 + Ed25519 verify (public key from env)
    │   ├── provision_http.rs POST /v1/provision → verify image → plan (Bearer)
    │   ├── tray.rs           system tray in Rust (no webview tray APIs)
    │   ├── commands.rs       the command surface exposed to the UI
    │   └── lib.rs            app setup and state
    └── tauri.conf.json
```

Detection, persistence, the Matchmaker client, and provisioner selection are
free of Tauri types (plain paths / structs), so a future background service can
reuse them without a window.

## Prerequisites

- **Node.js 20+** and npm.
- **Rust** (stable, 1.95 or newer): https://rustup.rs
- **Platform build dependencies** for Tauri v2 — see
  https://v2.tauri.app/start/prerequisites/
  - Windows: Microsoft C++ Build Tools and WebView2 (preinstalled on Windows 11).
  - Linux: `libwebkit2gtk-4.1-dev`, `build-essential`, `curl`, `wget`, `file`,
    `libxdo-dev`, `libssl-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`.
  - macOS (development only): Xcode Command Line Tools.

## Environment

Copy `.env.example` to `.env` on the host machine (never commit `.env`):

| Variable | Required | Purpose |
| --- | --- | --- |
| `MATCHMAKER_URL` | yes | Matchmaker origin, e.g. `http://127.0.0.1:8000` |
| `HOST_BOOTSTRAP_TOKEN` | yes | Same Bearer token as Matchmaker `HOST_BOOTSTRAP_TOKEN` |
| `HOST_DISPLAY_NAME` | no | Defaults to this machine's hostname |
| `HOST_AGENT_LISTEN_ADDR` | no | Bind `host:port` for `POST /v1/provision`. Omit to disable. If set, the bootstrap token **and** image trust env vars are required (fail fast). |
| `HOST_AGENT_URL` | no | Origin advertised as `agentUrl`. Defaults to `http://$HOST_AGENT_LISTEN_ADDR` when listening. |
| `IMAGE_SIGNING_PUBLIC_KEY` | when listening | 32-byte Ed25519 public key (64 hex chars). No private key on the agent. |
| `IMAGE_LOCAL_DIGEST` | when listening | SHA-256 hex of the standby image (or the Standard fixture for local stub). |
| `HEADSCALE_LOGIN_SERVER` | to join mesh | Headscale origin for `tailscale up --login-server`. Fail fast if join is attempted without it. Omit to skip join after register. |
| `TAILSCALE_BIN` | no | Defaults to `tailscale` on PATH. |

The agent fails registration and heartbeats if the URL or bootstrap token is
missing when it would send. It does not read `HOST_APPROVE_TOKEN`. Operators
approve with that token on the Matchmaker (`POST /v1/hosts/{id}/approve` or
`python -m matchmaker.seed --approve-host-id`).

`tauri dev` / `cargo run` load `.env` from `host-agent/` or `host-agent/src-tauri/`.
A packaged install should set the same variables in the service environment.

### Generate a test keypair (do not commit the PEM)

Ed25519 over the **raw 32-byte SHA-256 digest** (not the hex string):

```bash
openssl genpkey -algorithm ED25519 -out platform-ed25519.pem
openssl pkey -in platform-ed25519.pem -pubout -outform DER | tail -c 32 | xxd -p -c 32
# IMAGE_SIGNING_PUBLIC_KEY

printf 'dcl-fixture-standard-v1\n' | shasum -a 256
# IMAGE_STANDARD_DIGEST / IMAGE_LOCAL_DIGEST

DIGEST_HEX=<64 hex chars from sha256>
echo "$DIGEST_HEX" | xxd -r -p > /tmp/image.digest
openssl pkeyutl -sign -inkey platform-ed25519.pem -rawin -in /tmp/image.digest | openssl base64 -A
# IMAGE_STANDARD_SIGNATURE
rm platform-ed25519.pem /tmp/image.digest
```

`.env.example` already has fixture values for those Standard/Pro strings so
local Matchmaker + agent can verify without a real guest disk. Replace them
before any production image. Never commit `.pem` / `.key` / `.env`.

## Running

```bash
cp .env.example .env   # then edit MATCHMAKER_URL and HOST_BOOTSTRAP_TOKEN
npm install
npm run tauri dev      # the real agent, reads this machine's hardware
```

```bash
npm run dev            # UI only, in a browser, with sample hardware
```

The browser mode exists so the UI can be worked on without the Rust toolchain.
It shows a banner, writes nothing to disk, and only simulates Matchmaker
registration. Heartbeats and the system tray run only in the packaged agent.
Pause/resume in preview still updates local presence immediately.

To see what the agent detects without opening a window:

```bash
cd src-tauri && cargo run -- --print-hardware
```

```json
{
  "cpu": { "model": "Apple M1", "logicalCores": 8, "physicalCores": 8 },
  "memory": { "totalBytes": 8589934592 },
  "gpus": [],
  "gpuNote": "GPU detection currently supports NVIDIA GPUs on Windows and Linux only."
}
```

## Tests

```bash
cd src-tauri
cargo test
```

Covers CPU model normalization, the GPU fallback path, recommended defaults,
clamping a stored config to a smaller machine, the config file round trip,
immediate pause (`set_sharing_enabled` only flips the flag), inventory payload
clamping (including GPU id when `shareGpu` is true), fail-fast credentials,
POST vs PUT once a host id is persisted, heartbeat skip/send paths (including
pause via `set_sharing_enabled`), token redaction in `Debug`, presence
(paused beats occupancy), and earnings GET (`credits accrued (not paid out)`).
Provision HTTP: Linux/Windows stub URLs, 401 without Bearer, macOS rejected,
bind round-trip with `hypervisorInvoked` false, occupancy marked only on
HTTP 200. Image verify: valid signature accepts; tampered bytes reject;
missing public key / local digest fail fast; HTTP mismatch does not boot.

```bash
npm run typecheck
```

## Where the config is written

Two files, in the platform app-config directory:

| File | Contents |
| --- | --- |
| `sharing-config.json` | v2 caps: `version`, `sharingEnabled`, `sharedLogicalCores`, `sharedMemoryBytes`, `shareGpu`, `sharedGpuId` |
| `host-identity.json` | Last Matchmaker `hostId` (and last known status) so the next save can `PUT /v1/hosts/{id}` |

| OS | Directory |
| --- | --- |
| Windows | `%APPDATA%\com.distributedcomputelab.hostagent\` |
| Linux | `~/.config/com.distributedcomputelab.hostagent/` |
| macOS | `~/Library/Application Support/com.distributedcomputelab.hostagent/` |

The window shows the sharing-config path at the bottom. Values are re-clamped
to the detected hardware every time the file is read, and again in Rust before
the Matchmaker request, because a config file can travel to a different machine
through a backup or profile sync.

v2 of the sharing file adds `sharedGpuId` (the NVML UUID, or `nvidia:{index}` if
the driver hid it). v1 files that only have `shareGpu` still load; the missing
id is filled with the first detected GPU. An availability schedule is not stored
yet — that field can be added later without renaming the GPU id.

Local `sharingEnabled` is not a Matchmaker field. Pause stops heartbeats on the
next tick; after 45s of silence the Matchmaker will not select this host and
will interrupt an occupied session (same as disconnect). Occupancy on the
Matchmaker stays on session indexes. The window and tray can still show
in-session after a successful stub provision — Matchmaker silence teardown does
not call the agent, so the local occupancy flag is not cleared.

## Platform support

Windows and Linux are the real targets. macOS builds and runs for development,
but GPU detection there always reports "No GPU detected" because NVML is
Windows/Linux only. The provisioner treats macOS (and any other OS) as an
explicit unsupported host: selection fails instead of aliasing to Firecracker.

GPU sharing in this UI is exclusive for the session: the selected card is
handed to the guest, and the host loses it until logout. CPU/RAM sliders are
a concurrent slice. Do not present them as the same kind of control. The same
exclusivity copy is shown when a GPU-sharing host is registered.

## Notes for the next phase

- Phase 15 is Storj + rclone in the guest. **Ask the human first** — it needs
  a Storj account. Hardware gates stay OPEN. Mesh join is the software
  contract; default-deny ACLs and platform exit-node egress are **14c** (see
  `docs/runbooks/headscale.md`). Still do not invoke a real hypervisor from
  this agent. On macOS, `POST /v1/provision` returns 400 (`UnsupportedHostOs`);
  Matchmaker then keeps the seeded `kasmUrl`. Local in-session occupancy is
  still not cleared when Matchmaker interrupts a session (no teardown port).
  Real host payouts are not in this agent; the ledger only accrues credits.
  This agent never reads `HEADSCALE_API_KEY`.
- The app icon is generated from code: `node scripts/generate-icon.mjs` writes
  `src-tauri/icons/source.png`, then `npm run tauri icon src-tauri/icons/source.png`
  derives every platform size. Replace the script when there is a real logo.
