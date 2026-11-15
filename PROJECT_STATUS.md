# Project status

Last updated: 2026-11-15, after a structure cleanup and bug-fix pass.
The previous, much longer status is kept in
`docs/archive/project-status-2026-11-14.md`.

Rule: nothing here is called "working" unless a person saw it on real
hardware. Everything else is "passes tests".

## In one paragraph

All the software pieces exist and their tests pass. Two real PCs have shown
a desktop to a second device over Wi-Fi using the LAN test kit, which
bypasses most of the product. The full path (browser → Matchmaker → Host
Agent → signed guest image → mesh → gateway) has never run end to end on
real machines. That is the next milestone; two security holes should be
fixed before it (see "Next steps").

## Tests (2026-11-15, macOS)

`bash scripts/check.sh` runs all of these.

| Suite | Result |
| --- | --- |
| Host Agent, Rust (`cargo test --lib`) | 323 passed |
| Matchmaker (`pytest`, Postgres) | 185 passed |
| Web (`vitest`) + typecheck | 54 passed |
| Scripts and guest image (`pytest`) | 45 passed |
| Gateway (`pytest`) | 16 passed |
| Lint (`ruff`) | clean |

No test starts a real VM, GPU bind, Headscale, or renterd. Those are
injected fakes.

## What has run on real hardware

| What | Status |
| --- | --- |
| LAN test kit, quick mode, Windows 10/11 PC (QEMU + WHPX) | Seen working 2026-11-14. Desktop opened from a Mac on the same Wi-Fi. |
| LAN test kit, quick mode, Arch Linux PC (QEMU + KVM) | Seen working 2026-11-14, after a manual firewall rule. |
| Changes made to the kit since then (virtio disk, VPN-safe address, firewall cleanup) | Not re-run on hardware. |
| Signed Packer guest image | Never built and booted on a host. |
| Host Agent provision path (Matchmaker tells the agent to boot a guest) | Never run on hardware. |
| Guest joining Headscale, mounting storage | Never run. |
| Public gateway across two real networks | Never run. |
| Pro tier (GPU passthrough: Linux VFIO, Windows Hyper-V) | Never run. |
| Platform server (Matchmaker, Headscale, renterd, gateway) | **Down since 2026-11-15:** the Azure Free Trial subscription is Disabled (credit or trial period ran out). Replaced by **Oracle Cloud Always Free** (decided 2026-11-15; account being created). The new server needs a fresh deploy and a new IP in DuckDNS, `.env.vps` and `host-agent/.env.friend`. No deploy has ever been recorded in this repo. |

Logs: `docs/test-logs/`.

## What each component can do

- **Matchmaker**: signup/login, host registration with manual approval,
  heartbeats, automatic host selection by tier and declared capacity, one
  active session per host and per user, credits ledger, session interrupt
  and rematch when a host goes silent for 45 s, Headscale key minting,
  per-user renterd bucket, gateway link tokens.
- **Host Agent**: hardware and capability detection, sharing sliders,
  pause, tray, registration and heartbeats, mesh join, signed-image fetch
  and verification, booting a guest per backend, GPU hand-over with
  rollback, waiting for the desktop streamer before reporting success.
- **Web**: signup/login, task description → tier, automatic booking, link
  to the desktop, end session, history and credits.
- **Gateway**: exchanges a short-lived session link for a cookie and
  proxies HTTP and WebSocket to the guest inside the mesh.
- **Guest image**: Packer build of Ubuntu 24.04 with XFCE + KasmVNC
  (Standard) or Sunshine (Pro), signing, and the boot-time join script.

Not built: payments (credits are recorded only), latency-based ranking,
per-session mesh isolation, host schedules, direct peer-to-peer
streaming (FR17; today all desktop traffic goes through the VPS gateway),
the "Hide my IP" / "Fastest connection" choice (FR18), volunteer relay
nodes (FR19), Cloudflare R2 / Storj storage, Google Drive / Dropbox, and a
"My files" download/upload page (FR20).

## Changed on 2026-11-15

Bugs fixed (in code; hardware re-test still needed where noted):

1. Windows guest disk used emulated IDE in the Host Agent. Now virtio, as
   on Linux. (`docs/issues.md` 9)
2. Guests could never read their bootstrap token on QEMU hosts, so they
   never joined the mesh or mounted storage. The agent now passes it as
   SMBIOS strings and the guest script reads those first. The signed image
   must be rebuilt to include the new script. (issue 3)
3. Windows host setup told users to enable virtualization in the BIOS when
   it was already on. (issue 7)
4. A Matchmaker test had been failing since the ACL file gained the
   host → guest streamer-port rule; the old status file reported the suite
   as green. The test now checks that exactly ports 8443 and 47990 are
   allowed.

Structure:

- **Firecracker removed.** Linux Standard runs QEMU+KVM only, as
  production already did. Deleted: `firecracker.rs`, the TAP networking
  code, the Firecracker download in `hypervisor.rs`, the `Firecracker`
  backend and image variants, the unused `kernelPath` launch field,
  `FIRECRACKER_KERNEL`, `guest-images/scripts/extract_kernel.sh`, and the
  `flate2`/`tar` crates. About 2,300 lines of Rust in all. The Linux
  Standard provision tests that only covered Firecracker now cover
  QEMU+KVM; before this, that path had no end-to-end provision test. The
  architecture spec and the cahier now say QEMU+KVM, with the reason. The
  hardware gate runbook is `docs/runbooks/gate-kvm.md` (was `gate-fc.md`).
- Rust modules renamed after what they do: `qemu.rs` → `qemu_whpx.rs`
  (Windows Standard), `qemu_linux.rs` → `qemu_vfio.rs` (Linux Pro),
  `guest_net.rs` → `guest_bootstrap.rs` (only the bootstrap argv was
  left). `qemu_kvm.rs` (Linux Standard) unchanged.
- New root `README.md` with the component map.
- New `scripts/check.sh` and `.github/workflows/ci.yml` (tests on every
  push). Pushed to GitHub 2026-11-15; its first run was queued, result not
  yet checked.
- Real server details (VPS IP, user, DuckDNS domain) were scrubbed from
  the public repo and live in the local, git-ignored `.env.vps`;
  `scripts/vps-deploy.sh` loads it.
- Root tidied: `DEMO_RUNBOOK.md` → `docs/runbooks/demo.md`,
  `.vps-deploy.sh` → `scripts/vps-deploy.sh`, `PHASE_PLAN.md` and
  `PROJECT_CONTEXT.md` → `docs/archive/`.

## Next steps, in order

### 1. Fix the two security holes before anyone outside tests this

- Issue 1: every guest receives the platform's master storage key. A host
  owner can read it and then read every user's files. Needs a per-user or
  per-session key limited to that user's bucket.
- Issue 2: all hosts share one token, and the Matchmaker uses the same
  token to command agents. Anyone with a host kit can impersonate the
  Matchmaker to other hosts. Needs a per-host credential issued on
  approval.
- **Move platform storage to Cloudflare R2 at the same time** (decision
  2026-11-15: R2 now, Storj later, Sia/renterd dropped so no crypto
  payments are needed). R2 is S3-compatible, so the Matchmaker and the
  guest mostly need new settings. Check whether R2 API tokens limited to
  one bucket can be created per user; if so, that is the fix for issue 1.
  Add rclone encryption before upload and the per-user quota (20 GB free,
  more paid in credits). You create the Cloudflare account and the R2
  bucket/API token; renterd leaves `scripts/vps-deploy.sh`.

### 2. Direct peer-to-peer streaming (cahier FR17, added 2026-11-15)

Today every desktop frame goes browser → VPS gateway → mesh → guest and
back, so lag depends on where the VPS is, and all stream bandwidth is
paid for on the VPS. Goal: the stream goes **directly** between the
renter's browser and the guest; the platform carries only control
traffic (login, matching, signaling, heartbeats).

Plan:

1. Guest image: add Selkies-GStreamer (WebRTC desktop streaming, GPU
   encode for Pro) next to KasmVNC.
2. Matchmaker: a signaling endpoint protected by the session token.
3. VPS: a coturn TURN relay in `scripts/vps-deploy.sh` for sessions with
   no direct path (CGNAT, mobile networks, strict firewalls).
4. Host Agent: forward one UDP port into the guest (QEMU SLIRP) so WebRTC
   can cross the double NAT.
5. Web: a streaming page that tries direct, then TURN, then falls back to
   the existing HTTPS gateway (KasmVNC), automatically per session.
6. Privacy choice (cahier FR18): a setting for renters (web) and hosts
   (Host Agent), "Hide my IP" (default: platform relay only, nobody
   learns the other's IP) or "Fastest connection" (direct allowed). The
   Matchmaker allows direct only when both sides chose "Fastest
   connection" and tells the browser and guest which path to use.
7. Docs: host agreement and user terms explain both options in plain
   words.

Steps 1–6 are code; step 7 needs your wording and, later, a lawyer. The real test needs two
separate networks (host on home Wi-Fi, renter on phone data), measuring
lag against the gateway path and how often the relay is used. Not covered
by FR17: renter files (rclone ↔ storage) and guest internet (exit node)
still use platform infrastructure.

### 3. First full end-to-end run on real machines

First set up the Oracle Cloud server (see the hardware table). Then follow
`docs/runbooks/demo.md`. In order: build and sign the Standard image on a
Linux KVM machine (`guest-images/`), put its digest and signature into
the Matchmaker `.env`, deploy the VPS, install the Host Agent on one Linux
and one Windows PC, book a session from a browser on a different network.
This is the first time the bootstrap fix, the mesh, storage, and the
gateway (or direct streaming, if step 2 is done) run for real. Expect new
bugs; record them in `docs/issues.md` and the run in `docs/test-logs/`.

### 4. Make booking asynchronous (issue 4)

`POST /v1/sessions` blocks for up to 330 s while the VM boots. Return 202
with a `provisioning` state and let the web page poll. How long the signed
image takes to boot on real hardware is not measured yet; step 3 will show
whether 330 s is even enough.

### 5. User storage choices (cahier FR20)

- **"My files" page** in the web app: list, download (one file or all as a
  zip, also offered at logout) and upload, without starting a session.
- **Google Drive and Dropbox** as alternatives to platform storage: the
  user connects their account once (OAuth); the session mounts only an
  app folder in it (Drive `drive.file`, Dropbox "App folder"), never the
  whole account. Encryption is optional there.
- **Keep the whole home folder** (settings, browser profile, user-level
  apps), not only `~/Persistent`, plus a list of installed packages to
  reinstall at the next boot.

Needs from you: a Google Cloud project with an OAuth consent screen
(Google reviews it before public use) and a Dropbox app registration.

### 6. Re-run the LAN kit on both PCs

Confirms the virtio disk speed-up on Windows and closes issues 9, 11, 12,
14. Also find out why the desktop sometimes does not return on a second
run (issue 3b).

### 7. Growth phase (later, cahier phase 5)

Only once there are real users in more than one area:

- **Storj** as the decentralized platform storage next to R2, paid in
  normal money. Prefer rclone's native Storj mode (guest talks to the
  storage nodes directly, no gateway), which copes best with distance.
  Store each user's files in a region near where they usually work.

- **Regional platform relays:** cheap-bandwidth relay servers near where
  users are, so "Hide my IP" sessions stay fast.
- **Volunteer relay nodes (cahier FR19):** hosts with a public IP and good
  upload can opt in to relay other sessions' encrypted streams for
  credits per GB. The Matchmaker picks a relay near both sides and scores
  relays for reliability. Used only for "Fastest connection" sessions,
  because a relay sees both IPs. Reuses the Host Agent, the Matchmaker
  and the credits ledger; no blockchain needed.

### 8. Smaller items

- Reuse a session's mesh keys instead of minting new ones on every repeat
  request (issue 5).
- Split the three largest Rust files: `provision_http.rs` (2,900 lines),
  `gpu_bind.rs` (2,700), `image.rs` (1,900). About half of each is tests.
- Clear the remaining `cargo build` dead-code warnings (test-only helpers;
  `docs/issues.md` 6).
- Pro tier on real GPU hardware (`docs/runbooks/gate-pro.md`).
- Legal review of consumer GPU licences and host liability before any
  public launch.

## Things to know before changing code

- The Matchmaker will not start without a complete `.env`
  (`matchmaker/.env.example` lists every key).
- Pause is local to the agent: it stops heartbeats, so after 45 s the
  Matchmaker treats the host as gone and interrupts its session.
- Storage in the code today is renterd (Sia), on the zen test network,
  and every file passes through the VPS. Decided 2026-11-15: replace it
  with Cloudflare R2 (step 1), add Storj later; Sia mainnet is no longer
  planned.
- Mesh access is by role, not by session: any renter node can reach any
  guest node. The gateway link token is what scopes a renter to one
  session.
- macOS cannot host. On a Mac the agent runs but refuses to provision.
