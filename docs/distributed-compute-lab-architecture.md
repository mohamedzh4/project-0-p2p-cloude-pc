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
| **Compute plane** | Runs the actual virtual desktops | Host PCs, running Firecracker or QEMU/KVM |
| **Network plane** | Connects user ↔ VM ↔ storage securely | Tailscale / self-hosted Headscale |
| **Storage plane** | Persists user files across sessions | Storj, mounted via rclone |

Everything in this document fits into one of these four planes.

## 3. Two-Tier Lab Architecture

### 3.1 Standard Lab — office work, browsing, programming, light computing

| | |
|---|---|
| Isolation | **Firecracker** microVM (KVM-based, ~125ms cold boot, minimal device model → small attack surface) |
| Streaming | **Kasm Workspaces** (KasmVNC protocol — zero-install, browser-native) |
| Base image | Minimal Linux (Debian/Ubuntu minimal + lightweight desktop environment), Kasm agent pre-baked in |
| Why it fits | These workloads need CPU/RAM, not a GPU. Firecracker's tiny footprint lets one host run several concurrent sessions, and its near-instant boot makes an "always ready" standby image cheap to maintain. |

### 3.2 Pro/Gamer Lab — video editing, 3D rendering, gaming, AI

| | |
|---|---|
| Isolation | **KVM + QEMU** with **VFIO GPU passthrough** (near-native GPU performance) |
| Streaming | **Sunshine** (host) + **Moonlight** (client) — hardware-encoded via NVENC/AMF/QuickSync, purpose-built for low-latency, high-framerate video |
| Browser-native alternative | **Selkies-GStreamer** — open-source, WebRTC, GPU-accelerated, runs with zero client install. Worth evaluating alongside Moonlight, which traditionally expects a native client rather than a browser tab |
| OS | Windows (broadest compatibility) or Linux + Proton (no license cost, strong coverage — check ProtonDB per title before promising support) |
| Host requirements | IOMMU-capable CPU/motherboard (VT-d / AMD-Vi enabled), a GPU with clean VFIO reset behavior |

### Why the split is mandatory, not a design choice

Firecracker deliberately ships **no PCI passthrough support** — it's a minimal-device-model VMM built for security through simplicity, not a general-purpose hypervisor. There is no configuration flag that unlocks GPU passthrough on it. Splitting the stack at the GPU boundary isn't a shortcut; it's the only correct answer, and the two-tier structure you proposed already reflects that correctly.

## 4. Core Components & Tool Choices

| Layer | Tool | Role | Alternative worth knowing |
|---|---|---|---|
| MicroVM (Standard) | Firecracker | Hardware isolation, instant boot | Cloud Hypervisor (similar Rust VMM) |
| Full VM (Pro) | KVM + QEMU + VFIO | GPU passthrough | — this is already the standard approach |
| Desktop streaming | Kasm Workspaces | Browser-based remote desktop | Apache Guacamole (more manual assembly required) |
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
5. Host Agent boots the relevant VM from its standby base image — Firecracker for Standard, QEMU+VFIO for Pro
6. If the user has existing files, their Storj bucket is mounted **inside the VM** via rclone — the decryption key is only ever available inside the isolated guest, never on the host's bare disk
7. The browser connects directly to the session over the Tailscale-secured stream (KasmVNC, or Sunshine/Moonlight/Selkies depending on tier)
8. User works. Firecracker's near-zero overhead (Standard) or GPU passthrough's near-native performance (Pro) is what makes the session feel like the physical machine rather than a remote one
9. In the background, changed files sync to Storj automatically every 5–10 minutes, **plus immediately on an explicit save or on logout** — see §9 item 6 for why the fixed interval alone isn't enough
10. User logs out → Matchmaker triggers a final sync flush, then the VM and its ephemeral virtual disk are destroyed outright
11. Host returns to standby with a clean base image. Nothing the user did is recoverable from the host, because there was never a real file on its physical disk to recover

## 7. End-to-End Host Workflow

1. Host installs a lightweight **Host Agent** and logs in
2. Agent detects hardware; host sets a cap ("share up to 8 cores / 16GB / RTX 4070") and, optionally, an availability schedule
3. Agent automatically:
 - Requests a pre-auth key from the Matchmaker and joins the Tailscale/Headscale mesh
 - Registers its capacity with the Matchmaker
 - Downloads the correct **signed** base image for its resource class and keeps it on standby
4. Host UI stays minimal: idle / in-session / earnings — nothing to operate during a session
5. On match, the Matchmaker signals the Agent → Firecracker or QEMU+VFIO launches → storage mounts → streaming service starts → Agent reports "ready"
6. Host just needs power and a network connection for the session's duration
7. If the host disconnects mid-session, the Matchmaker's heartbeat monitor (missed pings for ~30–60s) flags the session as interrupted. Because of the sync cadence in §6.9, the user loses at most a few minutes of work and can be re-matched to a different host with their data intact
8. On a normal session end, the Agent wipes the ephemeral VM disk, returns to standby, and usage is reported for payout

## 8. Data & Storage Architecture

- **At rest:** Storj's client-side sharding and encryption mean no single storage node — or the host — ever holds plaintext
- **In transit:** every hop (user↔VM, VM↔storage) rides inside the WireGuard tunnel Tailscale already provides
- **On the host's physical disk:** rclone's local VFS cache should itself sit on an encrypted virtual disk (a LUKS-backed file, or simply the Firecracker/QEMU guest's own virtual block device) — so even the temporary cached fragments on the host's real drive are unreadable without a session key that lives only inside the guest and is discarded at teardown
- **Sync cadence:** the 5–10 minute background timer is a floor, not the only trigger. Pair it with an immediate flush on explicit "save" and on logout, so the worst-case data-loss window shrinks from "up to 10 minutes" to "since your last save"

## 9. Vulnerabilities, Gaps, and How to Patch Them

| # | Gap | Risk | Mitigation |
|---|---|---|---|
| 1 | Kasm's *default* isolation is Docker containers, which share the host kernel | Container escape could reach the host | Run Kasm's containers **inside** the Firecracker guest — Firecracker is the real isolation boundary, Kasm is just the streaming layer on top of it |
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

Items 5 and 10 are the two most likely to be missing from a first draft of a plan like this, and both are cheap to fix now and expensive to fix after launch.

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
- Onboarding: login, hardware detection, resource-sharing sliders, availability schedule
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

1. **Standard Lab only**, single region, hosts approved manually — wire Firecracker + Kasm + Tailscale + Storj together by hand to validate the core loop end to end
2. Automate host onboarding (the installer script), and build the real Matchmaker: matching logic + billing
3. Add the **Pro/Gamer Lab** (QEMU + VFIO + Sunshine), with latency-aware matching
4. Harden: base-image signing, exit-node routing, reliability scoring, and — if the market wants it — a confidential-computing tier

## 13. Open Questions Worth Tracking

- Anti-cheat compatibility is an industry-wide ceiling, not something this architecture can solve alone
- Confidential-computing hardware isn't universal on consumer PCs yet, so the pool of hosts eligible for "sensitive" sessions will be smaller than the general pool
- GPU vendor licensing (§9 item 10) needs a real legal review before commercial launch
- ToS, liability, and host indemnification need legal review before public launch
