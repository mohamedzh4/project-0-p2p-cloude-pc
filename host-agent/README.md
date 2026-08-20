# Host Agent

The program that runs on a volunteer's PC ("Host") in the Distributed Compute Lab.

This is phase 1. It does three things and nothing else:

1. Detects the machine's CPU, RAM and GPU.
2. Lets the host cap how much of each they are willing to share.
3. Saves that choice to a local config file.

There is no networking, no Matchmaker connection, no Firecracker or QEMU, and no
tray/background mode yet. See `../docs/distributed-compute-lab-architecture.md`
for where those land.

## Stack

- **Rust** core (`src-tauri/`) — all system access lives here.
- **Tauri v2** shell — a native webview instead of a bundled browser, so the
  agent stays small enough to sit on someone's daily-driver machine.
- **React + TypeScript + Vite** for the window contents.

`sysinfo` is asked for the narrowest refresh it supports (no process list, no
disks, no networks). NVIDIA GPUs are read through `nvml-wrapper`; anything else
reports "No GPU detected" with a note explaining why.

## Layout

```
host-agent/
├── src/                      UI
│   ├── App.tsx               window contents and state
│   ├── api.ts                the only place that calls Tauri commands
│   ├── components/           HardwareCard, ResourceSlider
│   ├── format.ts             byte and core formatting
│   ├── preview.ts            sample data for browser-only development
│   └── types.ts              mirrors the Rust models
└── src-tauri/
    ├── src/
    │   ├── models.rs         shared types, recommended defaults, clamping
    │   ├── hardware.rs       CPU + RAM detection
    │   ├── gpu.rs            NVIDIA detection with graceful fallback
    │   ├── config.rs         load/save the sharing config
    │   ├── commands.rs       the command surface exposed to the UI
    │   └── lib.rs            app setup and state
    └── tauri.conf.json
```

Detection and persistence are deliberately free of Tauri types (they take plain
paths and return plain structs), so a future background service can reuse them
without a window.

## Prerequisites

- **Node.js 20+** and npm.
- **Rust** (stable, 1.95 or newer): https://rustup.rs
- **Platform build dependencies** for Tauri v2 — see
  https://v2.tauri.app/start/prerequisites/
  - Windows: Microsoft C++ Build Tools and WebView2 (preinstalled on Windows 11).
  - Linux: `libwebkit2gtk-4.1-dev`, `build-essential`, `curl`, `wget`, `file`,
    `libxdo-dev`, `libssl-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`.
  - macOS (development only): Xcode Command Line Tools.

## Running

```bash
npm install
npm run tauri dev      # the real agent, reads this machine's hardware
```

```bash
npm run dev            # UI only, in a browser, with sample hardware
```

The browser mode exists so the UI can be worked on without the Rust toolchain.
It shows a banner and writes nothing to disk.

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
clamping a stored config to a smaller machine, and the config file round trip.

## Where the config is written

`sharing-config.json`, in the platform app-config directory:

| OS | Path |
| --- | --- |
| Windows | `%APPDATA%\com.distributedcomputelab.hostagent\sharing-config.json` |
| Linux | `~/.config/com.distributedcomputelab.hostagent/sharing-config.json` |
| macOS | `~/Library/Application Support/com.distributedcomputelab.hostagent/sharing-config.json` |

The window shows the exact path at the bottom. Values are re-clamped to the
detected hardware every time the file is read, because a config file can travel
to a different machine through a backup or profile sync.

v2 of the file adds `sharedGpuId` (the NVML UUID, or `nvidia:{index}` if the
driver hid it). v1 files that only have `shareGpu` still load; the missing id
is filled with the first detected GPU. An availability schedule is not stored
yet — that field can be added later without renaming the GPU id.

## Platform support

Windows and Linux are the real targets. macOS builds and runs for development,
but GPU detection there always reports "No GPU detected" because NVML is
Windows/Linux only. Hosting on macOS is a larger question than detection: the
Standard Lab is Firecracker on Linux and QEMU+WHPX on Windows, so a Mac host
would need a third provisioning path (QEMU, or Tart on Apple Silicon). That
decision belongs to the phase that adds provisioning, not this one.

GPU sharing in this UI is exclusive for the session: the selected card is
handed to the guest, and the host loses it until logout. CPU/RAM sliders are
a concurrent slice. Do not present them as the same kind of control.

## Notes for the next phase

- The app icon is generated from code: `node scripts/generate-icon.mjs` writes
  `src-tauri/icons/source.png`, then `npm run tauri icon src-tauri/icons/source.png`
  derives every platform size. Replace the script when there is a real logo.
- The Content-Security-Policy in `tauri.conf.json` allows no remote origins. Any
  future Matchmaker calls belong in Rust, not in `fetch` from the webview.
- `AgentState` currently holds only the hardware snapshot. Session state and a
  heartbeat task are the natural next things to hang off it.
