# Distributed Compute Lab — Complete Architecture Specification

## 1. Design Philosophy

**Integration over invention.** Every hard problem in this system — NAT traversal, encryption, GPU streaming, decentralized storage — has already been solved by a mature, audited, production-grade project. The only code this project truly needs to own is the **glue**: the Matchmaker service that ties these tools into one coherent product.

Four principles run through every decision below:

- **Hosts are zero-trust.** A host never sees plaintext user data and retains nothing after a session ends.
- **Guests are zero-trust.** A guest VM never sees the host's filesystem, LAN, or other guests' sessions.
- **Hosts are stateless.** All persistent data lives off-host, on decentralized storage. The host machine is disposable compute, not a data store.
- **Everything reusable is reused.** Custom code is written only where no existing tool fits.

## 2. The Four Planes

| Plane | Purpose | Owned by |
|---|---|---|
| **Control plane** | Auth, matching, billing, session lifecycle | Your custom Matchmaker service |
| **Compute plane** | Runs the actual virtual desktops | Host PCs that stay in daily use. Isolation is OS-conditional: Firecracker on Linux, QEMU+WHPX (or Hyper-V) on Windows; Pro sessions add GPU passthrough on top |
| **Network plane** | Connects user ↔ VM ↔ storage securely | Tailscale / self-hosted Headscale |
| **Storage plane** | Persists user files across sessions | Storj, mounted via rclone |

Everything in this document fits into one of these four planes.

## Implemented vs this paper (2026-09-04)

This file is the **target** architecture. Software on local `main` after
execution phase 7 is the **control-plane MVP with stubs**, not a live lab.

| Paper | In-repo now |
| --- | --- |
| Four planes | Control plane only (Matchmaker). Compute is OS-conditional **stubs**. Network/storage planes are not wired |
| Standard: Firecracker (Linux) / QEMU+WHPX (Windows) + Kasm in the guest | `plan_provision` + `POST /v1/provision` return `https://provision.invalid/{backend}`. No hypervisor, no guest image. GATE-FC / GATE-WHPX open |
| Pro: VFIO / Hyper-V GPU + Sunshine | Stubs require `sharedGpuId` and set `gpuExclusive`. GATE-PRO open |
| Intent capture, ranking, billing | FR2 harness maps intent → `labTier`; ranking still `SingleApprovedHostSelector`; no ledger |
| Tailscale/Headscale + exit-node egress | Not implemented. §9 item 5 still a launch blocker. Phase 14 — ask human first |
| Storj + rclone | Not implemented. Phase 15 — ask human first |
| Host Agent tray / mesh / signed images | Detection + caps + register/heartbeat + provision HTTP stub |

Do not treat a passing pytest/cargo suite as GATE-FC/WHPX/PRO.

## 3. Two-Tier Lab Architecture

### 3.1 Standard Lab — office work, browsing, programming, light computing

| | |
|---|---|
| Isolation | **OS-conditional.** Linux host → **Firecracker** microVM (KVM, ~125ms cold boot, minimal device model). Windows host → **QEMU with WHPX** (Windows Hypervisor Platform), or Hyper-V if WHPX is unavailable. Same isolation guarantee either way: the guest never sees the host filesystem or LAN. Firecracker does not run on Windows — there is no port, and hosts will commonly be ordinary Windows desktops — so the backend is chosen by OS, not by lab tier. |
| Streaming | **Kasm Workspaces** (KasmVNC protocol — zero-install, browser-native), running *inside* the guest |
| Base image | Minimal Linux (Debian/Ubuntu minimal + lightweight desktop environment), Kasm agent pre-baked in — the guest OS is independent of the host OS |
| Why it fits | These workloads need a CPU/RAM slice, not a GPU. The host keeps using the rest of the machine. On Linux, Firecracker's tiny footprint still lets one host run several concurrent Standard sessions; on Windows, QEMU+WHPX is heavier but avoids asking a volunteer to dual-boot or install Linux. |

### 3.2 Pro/Gamer Lab — video editing, 3D rendering, gaming, AI

| | |
|---|---|
| Isolation | Linux host → **KVM + QEMU** with **VFIO GPU passthrough**. Windows host → **Hyper-V Discrete Device Assignment** or **GPU-P**, or QEMU+WHPX with passthrough where the hardware allows it. Near-native GPU performance either way. |
| Streaming | **Sunshine** (inside the guest) + **Moonlight** (client) — hardware-encoded via NVENC/AMF/QuickSync, purpose-built for low-latency, high-framerate video |
| Browser-native alternative | **Selkies-GStreamer** — open-source, WebRTC, GPU-accelerated, runs with zero client install. Worth evaluating alongside Moonlight, which traditionally expects a native client rather than a browser tab |
| Guest OS | Windows (broadest compatibility) or Linux + Proton (no license cost, strong coverage — check ProtonDB per title before promising support) |
| Host requirements | Linux: IOMMU-capable CPU/motherboard (VT-d / AMD-Vi enabled) and a GPU with clean VFIO reset. Windows: a GPU the hypervisor can assign, plus the host still having a display path (iGPU or a second card) so they can keep using the PC. |

### Why the split is mandatory, not a design choice

Firecracker deliberately ships **no PCI passthrough support** — it's a minimal-device-model VMM built for security through simplicity, not a general-purpose hypervisor. There is no configuration flag that unlocks GPU passthrough on it. Splitting the stack at the GPU boundary isn't a shortcut; it's the only correct answer, and the two-tier structure you proposed already reflects that correctly.

The Standard/Pro split is independent of the Linux/Windows backend split. A Windows host still offers a Standard Lab (QEMU+WHPX, no GPU) and a Pro Lab (GPU assigned to the guest). A Linux host still offers Firecracker for Standard and QEMU+VFIO for Pro.

### 3.3 What "sharing" actually means on a machine the host still uses

Hosts keep using their PC. This is background resource sharing on a daily-driver machine, not a wipe-and-dedicate server.

**CPU and RAM are concurrent.** The Host Agent caps a slice for the guest (e.g. 8 of 16 cores, 16 of 32 GiB). The host OS and the host's own apps keep the rest. This is the default Standard Lab model.

**GPU sharing is not concurrent on consumer hardware.** Full passthrough (Linux VFIO, Windows Hyper-V DDA / GPU-P used as a whole-device assignment) hands that GPU to the guest for the session. The host loses it until the session ends. The realistic models for a consumer P2P network are:

1. The host has a **second GPU** (or an iGPU for their own display) and dedicates one card to sharing.
2. GPU sharing happens on a **schedule**, when the host is not using that GPU — not at the same time as their own GPU work.
3. Professional SR-IOV / vGPU cards that can partition a GPU. Out of scope here.

The Host Agent must present GPU sharing as that exclusive trade-off, not as a slider next to CPU and RAM. An availability schedule is a later feature; the schema should already record *which* GPU is dedicated so that schedule does not force another breaking change.

## 4. Core Components & Tool Choices

| Layer | Tool | Role | Alternative worth knowing |
|---|---|---|---|
| MicroVM (Standard, Linux host) | Firecracker | Hardware isolation, instant boot. Linux/KVM only — does not run on Windows | Cloud Hypervisor (similar Rust VMM, also Linux) |
| Full VM (Standard, Windows host) | QEMU + WHPX | Same guest isolation on a Windows desktop, no Linux install required on the host | Hyper-V as the hypervisor if WHPX is unavailable |
| Full VM (Pro, Linux host) | KVM + QEMU + VFIO | Exclusive GPU passthrough for the session | — this is already the standard approach |
| Full VM (Pro, Windows host) | Hyper-V DDA / GPU-P, or QEMU+WHPX with passthrough | Exclusive GPU assignment for the session; host needs another display path (iGPU or a second card) | — |
| Desktop streaming | Kasm Workspaces | Browser-based remote desktop, inside the guest | Apache Guacamole (more manual assembly required) |
| Game/GPU streaming | Sunshine + Moonlight | Low-latency hardware-encoded stream | Selkies-GStreamer (true browser-native, no client) |
| Network mesh | Tailscale / self-hosted Headscale | NAT traversal, WireGuard encryption, ACLs | Raw WireGuard + your own coordination server |
| Decentralized storage | Storj | S3-compatible, client-side sharded + encrypted | Filecoin (better for cold archival, weaker for a live mount) |
| Storage mount | rclone (`--vfs-cache-mode full`) | Local disk cache + async background sync | s3fs-fuse (weaker caching behavior) |
| Orchestration | Your Matchmaker | Everything above needs this to work together — see §5 | — must be custom-built |

## 5. The Matchmaker — This Is the Actual Product

None of the tools above talk to each other on their own. The Matchmaker is the service that:

1. **Owns identity** — logs the user in, resolves them to `User ID X`
2. **Captures intent** — "What do you want to do in your lab today?" maps a button or free-text answer to a lab tier, and for Pro, to a sub-requirement like VRAM size or a specific title
3. **Tracks inventory** — a real-time list of online hosts, their declared resources, current load, and a rolling reliability score
4. **Matches** — filters hosts by tier fit, ranks by latency (critical for Pro, more forgiving for Standard) and reliability, reserves the best one
5. **Drives provisioning** — calls the Host Agent to boot the VM, calls Headscale's API for a scoped ephemeral network identity, calls Storj/rclone to mount the user's bucket
6. **Watches the session** — heartbeats, triggers re-matching on host failure
7. **Bills** — meters usage, pays out hosts, charges users

This is the one component with no off-the-shelf substitute. Most of the engineering budget belongs here, not in re-implementing streaming or encryption.

## 6. End-to-End User Workflow

1. User opens the web app and logs in → resolved to `User ID X`
2. App asks: **"What do you want to do in your lab today?"**
3. Matchmaker maps the answer to Standard or Pro/Gamer, plus any GPU/VRAM requirement
4. Matchmaker finds the best available host and reserves it
5. Host Agent boots the relevant VM from its standby base image — Firecracker (Linux) or QEMU+WHPX (Windows) for Standard; QEMU+VFIO or Hyper-V GPU assignment for Pro
6. If the user has existing files, their Storj bucket is mounted **inside the VM** via rclone — the decryption key is only ever available inside the isolated guest, never on the host's bare disk
7. The browser connects directly to the session over the Tailscale-secured stream (KasmVNC, or Sunshine/Moonlight/Selkies depending on tier)
8. User works. The guest is isolated; the host keeps using the rest of their PC. Firecracker's near-zero overhead (Standard on Linux) or GPU passthrough's near-native performance (Pro) is what makes the session feel like a physical machine rather than a remote one. If this is a Pro session, the dedicated GPU is unavailable to the host until logout.
9. In the background, changed files sync to Storj automatically every 5–10 minutes, **plus immediately on an explicit save or on logout** — see §9 item 6 for why the fixed interval alone isn't enough
10. User logs out → Matchmaker triggers a final sync flush, then the VM and its ephemeral virtual disk are destroyed outright
11. Host returns to standby with a clean base image. Nothing the user did is recoverable from the host, because there was never a real file on its physical disk to recover

## 7. End-to-End Host Workflow

1. Host installs a lightweight **Host Agent** on the PC they already use, and logs in
2. Agent detects hardware; host sets a CPU/RAM cap they can spare while they keep working, and optionally dedicates a specific GPU (knowing they lose that GPU for the session). An availability schedule is optional and comes later.
3. Agent automatically:
 - Requests a pre-auth key from the Matchmaker and joins the Tailscale/Headscale mesh
 - Registers its capacity with the Matchmaker
 - Downloads the correct **signed** base image for its resource class and keeps it on standby
4. Host UI stays minimal: idle / in-session / earnings — nothing to operate during a session. The host keeps using the machine.
5. On match, the Matchmaker signals the Agent → the OS-appropriate backend launches (Firecracker, QEMU+WHPX, or QEMU+VFIO / Hyper-V GPU assignment) → storage mounts → streaming service starts → Agent reports "ready"
6. For a Standard session the host keeps working as usual, minus the capped CPU/RAM slice. For a Pro session they also lose the dedicated GPU until the session ends.
7. If the host disconnects mid-session, the Matchmaker's heartbeat monitor (missed pings for ~30–60s) flags the session as interrupted. Because of the sync cadence in §6.9, the user loses at most a few minutes of work and can be re-matched to a different host with their data intact
8. On a normal session end, the Agent wipes the ephemeral VM disk, returns the GPU to the host if one was assigned, returns to standby, and usage is reported for payout

## 8. Data & Storage Architecture

- **At rest:** Storj's client-side sharding and encryption mean no single storage node — or the host — ever holds plaintext
- **In transit:** every hop (user↔VM, VM↔storage) rides inside the WireGuard tunnel Tailscale already provides
- **On the host's physical disk:** rclone's local VFS cache should itself sit on an encrypted virtual disk (a LUKS-backed file on Linux, a BitLocker-backed file on Windows, or simply the guest's own virtual block device) — so even the temporary cached fragments on the host's real drive are unreadable without a session key that lives only inside the guest and is discarded at teardown
- **Sync cadence:** the 5–10 minute background timer is a floor, not the only trigger. Pair it with an immediate flush on explicit "save" and on logout, so the worst-case data-loss window shrinks from "up to 10 minutes" to "since your last save"

## 9. Vulnerabilities, Gaps, and How to Patch Them

| # | Gap | Risk | Mitigation |
|---|---|---|---|
| 1 | Kasm's *default* isolation is Docker containers, which share the host kernel | Container escape could reach the host | Run Kasm's containers **inside** the guest VM — Firecracker (Linux) or QEMU/Hyper-V (Windows) is the real isolation boundary, Kasm is just the streaming layer on top of it |
| 2 | A malicious host could tamper with the base image before serving it | Malware or a keylogger injected before the user ever connects | Sign every base image with a platform key; the Agent and hypervisor verify the hash before every boot, and reject and re-pull on mismatch |
| 3 | No confidential computing by default | A host with physical access could theoretically inspect a running guest's memory | Route sessions flagged as sensitive to hosts with AMD SEV-SNP / Intel TDX when available; otherwise be explicit in your ToS about the trust model |
| 4 | Overly broad Tailscale ACLs | A guest VM could reach the host's other LAN devices, or another user's session | Ephemeral, tagged, **per-session** tailnet identities; default-deny ACLs allowing only `VM ↔ Matchmaker` and `VM ↔ assigned user` |
| 5 | **VM internet traffic exits through the host's home IP** | Anything a user does online appears to come from the host's own ISP account — real legal exposure for an innocent volunteer | Route all guest egress through a Tailscale **exit node** you operate (a small fleet of relay IPs) — never through the host's raw connection. This is a small addition to infrastructure you already run, not a new system |
| 6 | Fixed 5–10 minute backup interval | Up to ~10 minutes of work lost on an abrupt disconnect | Add event-triggered flushes (save, logout) on top of the timer, and show the user a "saved as of HH:MM" indicator so the risk is visible, not hidden |
| 7 | Kernel-level anti-cheat (Easy Anti-Cheat, BattlEye, Vanguard) often blocks virtualized or streamed sessions outright | A meaningful share of competitive multiplayer titles simply won't launch | This is the anti-cheat vendor's policy, not a bug to route around — publish a compatibility list and scope initial gaming support to titles that work. Every cloud-gaming service has this same limitation |
| 8 | Hosts could misreport their hardware | Wasted matches, poor user experience | Run an automated benchmark during onboarding and periodically re-verify |
| 9 | Session tokens could be replayed or shared | Someone rides along on a session that isn't theirs | Short-lived, single-use tokens bound to the requesting device; TLS/WireGuard everywhere |
| 10 | Consumer GPU driver EULAs | NVIDIA/AMD consumer driver licenses have historically restricted datacenter or commercial "cloud gaming"-style use of consumer cards | Get this reviewed by a lawyer before commercial launch — it may push power users toward workstation-class cards, or shape how the ToS is written |
| 11 | No liability framework | Undefined responsibility if illegal content is processed on a volunteer's hardware | Acceptable-use policy, host indemnification clause, and — before public launch — an actual legal review |
| 12 | Consumer GPU passthrough is exclusive, not a slice | A host who enables GPU sharing loses that GPU (display, their own games/apps) for the whole session | Be explicit in the Host Agent UI; require a second GPU or iGPU, or a schedule when the host is away. Do not present GPU sharing as concurrent the way CPU/RAM are. SR-IOV/vGPU is out of scope |

Items 5, 10, and 12 are the ones most likely to be missing from a first draft, and all three are cheap to fix now and expensive to fix after launch.

## 10. Reliability & Failure Handling

- Host Agent → Matchmaker heartbeat every few seconds; roughly 30–60s of silence marks a session interrupted
- Because of the layered sync strategy in §8, the data-loss window on disconnect is small and bounded, not "whatever was in progress"
- A host's disconnect frequency should feed into its reliability score, so the Matchmaker naturally routes more sessions toward hosts that stay up

## 11. Application & Website Structure

**User-facing web app**
- Landing / marketing page
- Login (credentials → `User ID X`)
- Dashboard: the "What do you want to do in your lab today?" prompt, recent sessions, stored files, credits/billing
- Session view: full-screen embedded stream (KasmVNC or Sunshine/Moonlight/Selkies canvas), connection-quality indicator, save/logout controls
- Account settings: payment, privacy controls, session history

**Host-side app** (a lightweight tray app — Tauri or Electron — not a web page)
- Onboarding: login, hardware detection, CPU/RAM sliders (concurrent with the host's own use), GPU picker with an exclusivity warning, availability schedule later
- Status view: idle / in-session / earnings, nothing to operate mid-session — the host is still using the PC
- Status view: idle / in-session / earnings, nothing to operate mid-session
- Settings: pause sharing, view logs, uninstall

**Backend services the Matchmaker needs to own or run**
- Auth service
- Matching/scheduling engine
- Host inventory + heartbeat tracking
- Billing/ledger
- Self-hosted Headscale instance
- Base-image signing & registry
- Storj account/bucket provisioning
- A small fleet of Tailscale exit nodes (for §9 item 5)
- Basic observability (host health, session quality) — a Prometheus + Grafana stack is a standard, well-understood fit

## 12. Suggested Build Order

1. **Standard Lab only**, single region, hosts approved manually — wire the OS-conditional backend (Firecracker+Kasm on Linux, QEMU+WHPX+Kasm on Windows) + Tailscale + Storj together by hand to validate the core loop end to end
2. Automate host onboarding (the installer script), and build the real Matchmaker: matching logic + billing
3. Add the **Pro/Gamer Lab** (QEMU+VFIO or Hyper-V GPU assignment + Sunshine), with latency-aware matching and honest GPU exclusivity
4. Harden: base-image signing, exit-node routing, reliability scoring, and — if the market wants it — a confidential-computing tier

## 13. Open Questions Worth Tracking

- Anti-cheat compatibility is an industry-wide ceiling, not something this architecture can solve alone
- Confidential-computing hardware isn't universal on consumer PCs yet, so the pool of hosts eligible for "sensitive" sessions will be smaller than the general pool
- GPU vendor licensing (§9 item 10) needs a real legal review before commercial launch
- Consumer GPU sharing is exclusive for the session (§3.3, §9 item 12); a second GPU or a later availability schedule is what makes Pro hosting realistic on a daily-driver PC
- ToS, liability, and host indemnification need legal review before public launch
