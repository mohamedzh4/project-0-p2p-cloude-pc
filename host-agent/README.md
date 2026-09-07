# Host Agent

The program that runs on a volunteer's PC ("Host") in the Distributed Compute Lab.

This is the Host Agent after execution phase 31 (Linux Standard Firecracker
invoke + Windows Standard QEMU `-accel whpx` + Linux Pro VFIO bind apply +
Windows Pro Hyper-V DDA/GPU-P apply + live Kasm/Sunshine locator poll). It
does these things and nothing else:

1. Detects the machine's CPU, RAM and GPU, and whether Standard/Pro sessions
   can be offered (OS + KVM / Windows hypervisor / IOMMU / second display
   path). The UI never asks which backend to use.
2. Lets the host cap how much of each they are willing to share.
3. Saves that choice to a local config file (v2).
4. Registers those **clamped** caps with the Matchmaker from **Rust**
   (`POST /v1/hosts` or `PUT /v1/hosts/{id}`). New hosts stay `pending` until an
   operator approves them with `HOST_APPROVE_TOKEN`. This agent never sends that
   token. When `HOST_AGENT_LISTEN_ADDR` is set, registration includes `agentUrl`.
5. While local `sharingEnabled` is on and a host id exists, pings
   `POST /v1/hosts/{id}/heartbeat` from Rust every 10 seconds. Pause sharing to
   stop pings immediately (next tick) so the host goes stale in matching.
6. Selects a provision target from host OS × lab tier (Linux Standard
   → Firecracker, Windows Standard → QEMU+WHPX,    Pro → QEMU+VFIO / Hyper-V GPU).
   macOS is an explicit unsupported variant. `plan()` never invokes a
   hypervisor. Linux Standard `POST /v1/provision` starts Firecracker when
   the probe offers Standard and the binary, kernel, and verified rootfs
   are present (`InstanceStart` over a Unix socket). Windows Standard
   `POST /v1/provision` starts QEMU with documented `-accel whpx` when the
   probe offers Standard and the binary plus a verified disk are present.
   Linux Pro `POST /v1/provision` applies vfio-pci bind (rollback on
   failure) when the probe offers Pro. Windows Pro `POST /v1/provision`
   applies Hyper-V DDA (preferred) or exclusive GPU-P (rollback on failure)
   when the probe offers Pro. The settings window stays plan-only
   (`apply: false`, `hyperv: None`) so opening it does not steal the GPU.
   The Hyper-V VM `dcl-pro-guest` must already exist and be powered off;
   this agent does not create or `Start-VM` it. After a successful invoke or
   GPU apply, provision polls for a reachable Kasm (`https://{meshHost}/`,
   443) or Sunshine (`https://{meshHost}:47990/`) locator and returns that
   URL, or 400 if the streamer never answers. Tests inject loopback HTTP.
   Plan-only stubs may still return `https://provision.invalid/{backend}`.
7. Optionally listens for `POST /v1/provision` from the Matchmaker
   (`HOST_AGENT_LISTEN_ADDR`). The handler verifies the advertised SHA-256
   digest + Ed25519 signature against `IMAGE_SIGNING_PUBLIC_KEY` and
   `IMAGE_LOCAL_DIGEST`, then calls `plan_provision`. Linux Standard with
   boot inputs invokes Firecracker or returns 400 (no fake success). Windows
   Standard with boot inputs invokes QEMU `-accel whpx` or returns 400.
   Linux Pro with VFIO inputs applies vfio-pci or returns 400 (rollback on
   mid-step failure). Windows Pro with Hyper-V inputs applies DDA/GPU-P or
   returns 400 (rollback on mid-step failure; tests never spawn PowerShell).
   After invoke/apply, the locator is the reachable streamer URL (mesh
   hostname, or injected loopback in tests). Timeout or a missing mesh
   hostname is 400 — not a `.invalid` success. Mismatch is 400.
   Complete storage locators yield `guestRclone` (`--s3-env-auth`); guest-mesh
   hints are accepted and not used for host `tailscale up`. The webview does
   not `fetch` this.
8. Shows local presence: paused / idle / in-session. Earnings are
   `{n} credits accrued (not paid out)` from `GET /v1/hosts/{id}/earnings`
   (Rust, not the webview). Zero if this machine has no host id yet.
9. If `HEADSCALE_LOGIN_SERVER` is set, after register it POSTs
   `/v1/hosts/{id}/mesh/preauth` from Rust and runs `tailscale up` with that
   login-server (env) and the minted key. Missing mesh/Matchmaker env fails
   fast. It does **not** pass `--advertise-exit-node` or `--exit-node`. Omit the login-server
   env to skip join (UI-only).
10. Sits in the system tray (Rust, not the webview): tooltip, pause/resume,
    open window, quit. Closing the window hides to the tray when the icon
    installed.
11. After the capability probe, locates or fetches the OS hypervisor **binary**
    into a user cache (Linux Firecracker, documented GitHub URL + SHA-256).
    Windows Standard fetches the documented Weilnetz NSIS installer, verifies
    SHA-512, and runs it with `/S` and `/D=<cache>` (NSIS Chapter 3). UAC may
    still appear (`qemu.nsi` `RequestExecutionLevel admin`) and cannot be
    suppressed. macOS skips with a reason. Locate/fetch never execs the
    binary. Linux Standard provision (item 6) starts Firecracker from
    `firecracker.rs`. Windows Standard provision starts QEMU from `qemu.rs`.
    The webview does not download it. Tests inject a fake
    installer and a fake Firecracker/QEMU process.
12. Plans Pro GPU assignment from the dedicated GPU id: Linux vfio-pci
    (injectable sysfs, rollback on apply failure); Windows Hyper-V DDA
    (preferred) or exclusive GPU-P via an injectable PowerShell/CIM runner
    (rollback on apply failure). The volunteer does not run `lspci`.
    The window does not bind a live GPU. Linux Pro bind and Windows Pro
    Hyper-V DDA/GPU-P apply on Matchmaker `POST /v1/provision` (tests inject
    a mock tree / fake runner and refuse live `/sys` / `powershell.exe`).

13. Combines the capability probe, the 2 vCPU / 4 GiB / 50 GiB floor, and a
    refused VFIO/DDA plan into one Standard/Pro offer. A blocked tier is not
    offered (plain-language reason; no launch document; live provision is
    400). GPU exclusivity copy stays when Pro *is* offered.

There is no live Kasm or Sunshine session in this repo. Linux Standard can
`InstanceStart`; Windows Standard can spawn `-accel whpx`; Linux Pro can apply
vfio-pci from provision; Windows Pro can apply Hyper-V DDA/GPU-P from
provision; locators stay `.invalid` (no TAP/net). See
`../docs/distributed-compute-lab-architecture.md` for where those land.

## Stack

- **Rust** core (`src-tauri/`) — all system access and Matchmaker HTTP live here.
- **Tauri v2** shell — a native webview instead of a bundled browser, so the
  agent stays small enough to sit on someone's daily-driver machine.
- **React + TypeScript + Vite** for the window contents.

`sysinfo` is asked for the narrowest refresh it supports (no process list, no
networks) when detecting CPU/RAM. NVIDIA GPUs are read through `nvml-wrapper`;
anything else reports "No GPU detected" with a note explaining why. Spare-disk
eligibility only asks free space on the image-cache volume — not a disk inventory.

The Content-Security-Policy allows no remote origins. Matchmaker calls belong
in Rust, not `fetch` from the webview.

## Layout

```
host-agent/
├── .env.example              MATCHMAKER_URL + HOST_BOOTSTRAP_TOKEN
├── src/                      UI
│   ├── App.tsx               window contents and state
│   ├── api.ts                the only place that calls Tauri commands
│   ├── components/           HardwareCard, CapabilityPanel, ResourceSlider, GpuShareField
│   ├── format.ts             byte and core formatting
│   ├── preview.ts            sample data for browser-only development
│   └── types.ts              mirrors the Rust models
└── src-tauri/
    ├── src/
    │   ├── models.rs         shared types, recommended defaults, clamping
    │   ├── hardware.rs       CPU + RAM detection
    │   ├── gpu.rs            NVIDIA detection with graceful fallback
    │   ├── host_capabilities.rs  OS + KVM/WHPX/IOMMU/GPU probe (no Tauri; no hypervisor)
    │   ├── hypervisor.rs     locate/fetch Firecracker (Linux) / QEMU NSIS (Windows); never exec here
    │   ├── firecracker.rs    Linux Standard InstanceStart over Unix API (injected in tests)
    │   ├── qemu.rs           Windows Standard `-accel whpx` (injected in tests)
    │   ├── config.rs         load/save the sharing config; immediate pause
    │   ├── identity.rs       persisted Matchmaker host id
    │   ├── matchmaker.rs     inventory + heartbeat + earnings GET (clamp, fail-fast env)
    │   ├── mesh.rs           Matchmaker preauth mint + tailscale up from env (no secrets in source; never --exit-node)
    │   ├── storage.rs        Guest rclone argv/plan from locators or env (no S3 secrets, no spawn)
    │   ├── agent_status.rs   paused / idle / in-session (no Tauri types)
    │   ├── provision.rs      OS-conditional stub backends (plan() never invokes)
    │   ├── image.rs          SHA-256 + Ed25519 verify; locate/fetch signed guest image (Linux + Windows Standard; never boot)
    │   ├── launch.rs         clamped sliders → launch-config JSON (document only)
    │   ├── eligibility.rs    probe + CPU/RAM/disk floor + VFIO plan → offered vs blocked
    │   ├── disk_space.rs     spare bytes on the image-cache volume (not a disk inventory)
    │   ├── gpu_bind.rs       VFIO plan; Linux Pro provision apply + rollback; Windows DDA/GPU-P apply from provision via injectable runner
    │   ├── provision_http.rs POST /v1/provision → verify image → plan; Linux/Windows Standard may invoke; Linux Pro may VFIO-bind; Windows Pro may Hyper-V-assign
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
| `IMAGE_STANDARD_DIGEST` / `IMAGE_STANDARD_SIGNATURE` | Linux fetch | Same Matchmaker `IMAGE_*` checksum metadata (`GET /v1/images/standard`). Required to fetch+verify on Linux when Standard is offered. |
| `IMAGE_PRO_DIGEST` / `IMAGE_PRO_SIGNATURE` | Linux Pro fetch | Same Matchmaker `IMAGE_*` metadata when Pro is offered. |
| `HOST_AGENT_IMAGE_CACHE` | no | User-writable dir for guest images. Default `~/.cache/dcl-host-agent/images` (Linux), `~/Library/Caches/dcl-host-agent/images` (macOS), `%LOCALAPPDATA%\dcl-host-agent\images` (Windows). |
| `HOST_AGENT_STANDARD_IMAGE_URL` / `HOST_AGENT_PRO_IMAGE_URL` | Linux fetch | Bytes URL for the signed image. Not a secret. Matchmaker does not host blobs. |
| `HOST_AGENT_IMAGE_NETWORK` | tests only | `1` allows non-loopback HTTP during `cargo test`. Leave unset in CI. |
| `HEADSCALE_LOGIN_SERVER` | to join mesh | Headscale origin for `tailscale up --login-server`. Fail fast if join is attempted without it. Omit to skip join after register. |
| `TAILSCALE_BIN` | no | Defaults to `tailscale` on PATH. |
| `HOST_AGENT_HYPERVISOR_CACHE` | no | User-writable dir for Firecracker and the Windows QEMU NSIS install. Default `~/.cache/dcl-host-agent/hypervisors` (Linux), `~/Library/Caches/dcl-host-agent/hypervisors` (macOS), `%LOCALAPPDATA%\dcl-host-agent\hypervisors` (Windows). |
| `HOST_AGENT_FIRECRACKER_URL` / `HOST_AGENT_FIRECRACKER_SHA256` | no | Override the documented GitHub tarball + published SHA-256. Not secrets. |
| `HOST_AGENT_QEMU_URL` / `HOST_AGENT_QEMU_SHA512` | no | Override the pinned Weilnetz NSIS installer + published SHA-512. `HOST_AGENT_QEMU_SHA256` is accepted for test fixtures. Not secrets. |
| `HOST_AGENT_QEMU_BIN` | no | Existing `qemu-system-*.exe` (skip fetch/install when present). |
| `HOST_AGENT_HYPERVISOR_NETWORK` | tests only | `1` allows non-loopback HTTP during `cargo test`. Leave unset in CI. |

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
npm run tauri build    # packaged binary for this OS (phase 33 owns cross-compile)
```

Artifacts land in `src-tauri/target/release/bundle/` (plus
`src-tauri/target/release/host-agent` / `host-agent.exe`). Copy the matching
OS artifact to a volunteer PC, set env from `.env.example`, then detect /
register / heartbeat / wait for provision. This Mac cannot emit Linux or
Windows installers. UAC still appears on Windows QEMU NSIS install.

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
bind round-trip; Linux Standard with boot inputs sets `hypervisorInvoked`
true and returns the injected live locator; missing kernel or unreachable
Kasm is 400 (occupancy unset). Windows Standard with boot inputs sets
`hypervisorInvoked` true and returns the live locator; missing QEMU/disk/WHPX
or unreachable Kasm is 400. Linux Pro with VFIO inputs applies bind and
returns the live Sunshine locator; mid-step failure or unreachable Sunshine
is 400 with rollback. Windows Pro with Hyper-V inputs applies DDA (preferred)
or exclusive GPU-P and returns the live Sunshine locator; mid-step failure
or unreachable Sunshine is 400 with rollback; tests inject `HyperVGpuOps`
and never spawn PowerShell. Occupancy marked only on HTTP 200. Image verify: valid signature accepts; tampered bytes reject;
missing public key / local digest fail fast; HTTP mismatch does not boot.
Hypervisor fetch: Linux missing → install into cache after SHA-256; present →
skip; checksum mismatch → fail fast and do not install; macOS skip; Windows
QEMU NSIS `/S` `/D=` (injected installer); tests use an injected fetcher, `file://`, or loopback
httptest (no GitHub download).

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

- Phase 32 finished the `web/` loop (clickable live locator, auto-match
  kept). Next is a **copyable Host Agent binary**: `npm run tauri build`
  from `host-agent/` (see Running). Cross-compile / signing / Windows+Linux
  artifacts from this Mac are still gaps.
- Phase 23 insufficient-resource UX is in: the window shows Standard/Pro as
  **Available** or **Not available** with a reason from the probe, the
  2 vCPU / 4 GiB / 50 GiB floor, or a refused VFIO/DDA plan. A blocked tier
  is not offered. Launch config does not emit; live `POST /v1/provision`
  returns 400 (no stub URL, occupancy unset). GPU exclusivity copy stays
  when Pro is offered. Capability probe still never asks which backend.
  Phase 22 VFIO/DDA bind remains plan-only from the window. Phase 29 applies
  Linux vfio-pci from Matchmaker provision against real sysfs (injectable in
  tests; live `/sys` refused under `cfg(test)`; rollback on mid-step failure).
  Phase 30 applies Windows Hyper-V DDA (preferred) or exclusive GPU-P from
  Matchmaker provision through an injectable runner (production PowerShell;
  tests refuse live `powershell.exe`; rollback on mid-step failure). The VM
  `dcl-pro-guest` must already exist and be powered off; this agent does not
  `Start-VM`. Firecracker is never used for Pro.
  Session guest/user tags live on Matchmaker JSON; this agent still joins as
  `tag:host` and never passes `--advertise-exit-node` or `--exit-node`.
  Complete provision locators return `guestRclone` argv (`--s3-env-auth`).
  Hardware is implemented in code, not yet run on physical hardware — still no
  live Kasm/Sunshine session in this repo. Guest disks: `guest-images/`. Storage
  is renterd S3, not Storj. On
  macOS, `POST /v1/provision` is 400 (not offered / unsupported OS);
  Matchmaker then keeps the seeded locator. Local in-session occupancy is
  still not cleared when Matchmaker interrupts a session (no teardown port).
  Real host payouts are not in this agent. This agent never reads
  `HEADSCALE_API_KEY` or renterd S3 secrets. Do not fake latency. Phase 24
  (leftover **19b**) runs the QEMU NSIS installer with `/S` `/D=`; UAC
  consent is still manual. Phase 25 (leftover **22b**) applies Hyper-V
  DDA/GPU-P through an injectable runner with rollback; live PowerShell is
  never spawned from tests. Phase 30 runs that apply from
  `POST /v1/provision` (window still plan-only). On metal, Hyper-V apply
  still needs an elevated agent, the Hyper-V feature, and an existing VM
  `dcl-pro-guest`. Phase 26
  Packer pipeline produces signed guest disks. Phase 27 InstanceStarts
  Firecracker on Linux Standard HTTP provision; phase 28 spawns QEMU
  `-accel whpx` on Windows Standard; locators stay `.invalid` (no TAP).
  Operator kernel is `FIRECRACKER_KERNEL` (Packer does not invent vmlinux).
- The app icon is generated from code: `node scripts/generate-icon.mjs` writes
  `src-tauri/icons/source.png`, then `npm run tauri icon src-tauri/icons/source.png`
  derives every platform size. Replace the script when there is a real logo.
