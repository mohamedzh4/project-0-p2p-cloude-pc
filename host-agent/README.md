# Host Agent

The program that runs on a volunteer's PC ("Host") in the Distributed Compute Lab.

This is execution phase 5. It does six things and nothing else:

1. Detects the machine's CPU, RAM and GPU.
2. Lets the host cap how much of each they are willing to share.
3. Saves that choice to a local config file (v2).
4. Registers those **clamped** caps with the Matchmaker from **Rust**
   (`POST /v1/hosts` or `PUT /v1/hosts/{id}`). New hosts stay `pending` until an
   operator approves them with `HOST_APPROVE_TOKEN`. This agent never sends that
   token.
5. While local `sharingEnabled` is on and a host id exists, pings
   `POST /v1/hosts/{id}/heartbeat` from Rust every 10 seconds. Pause sharing to
   stop pings so the host goes stale in matching.
6. Selects a **stub** provision target from host OS × lab tier (Linux Standard
   → Firecracker, Windows Standard → QEMU+WHPX, Pro → QEMU+VFIO / Hyper-V GPU).
   macOS is an explicit unsupported variant. No hypervisor is invoked.

There is no real Firecracker, QEMU, or tray mode yet. See
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
    │   ├── config.rs         load/save the sharing config
    │   ├── identity.rs       persisted Matchmaker host id
    │   ├── matchmaker.rs     inventory + heartbeat client (clamp, POST/PUT/ping, fail-fast env)
    │   ├── provision.rs      OS-conditional stub backends (peers; no hypervisor)
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

The agent fails registration and heartbeats if the URL or bootstrap token is
missing when it would send. It does not read `HOST_APPROVE_TOKEN`. Operators
approve with that token on the Matchmaker (`POST /v1/hosts/{id}/approve` or
`python -m matchmaker.seed --approve-host-id`).

`tauri dev` / `cargo run` load `.env` from `host-agent/` or `host-agent/src-tauri/`.
A packaged install should set the same variables in the service environment.

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
registration. Heartbeats run only in the packaged agent.

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
inventory payload clamping (including GPU id when `shareGpu` is true), fail-fast
credentials, POST vs PUT once a host id is persisted, heartbeat skip/send paths,
and token redaction in `Debug`.

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

Local `sharingEnabled` is not a Matchmaker field. Pause stops heartbeats; after
45s of silence the Matchmaker will not select this host. Occupancy stays on
session indexes — pause does not end an active session.

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

- Phase 6: Matchmaker-triggered mock provision. After `HostSelector.select`,
  call `host_agent_lib::plan_provision` (`provision::plan`) — do not put
  hypervisor names in the session router. Still do not invoke a real hypervisor.
- The app icon is generated from code: `node scripts/generate-icon.mjs` writes
  `src-tauri/icons/source.png`, then `npm run tauri icon src-tauri/icons/source.png`
  derives every platform size. Replace the script when there is a real logo.
