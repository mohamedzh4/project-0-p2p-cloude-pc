# Project status

Last updated: 2026-11-15, after a structure cleanup and bug-fix pass.
The previous, much longer status is kept in
`docs/archive/project-status-2026-11-14.md`.

Rule: nothing here is called "working" unless a person saw it on real
hardware. Everything else is "passes tests".

## In one paragraph

All the software pieces exist and their tests pass. Roadmap phases 1 and 2
are done in code (2026-11-15): the two security holes and the renter
isolation hole are closed, booking no longer blocks, and an integration
test drives a whole booking through the real Matchmaker and gateway. Two
real PCs have shown a desktop to a second device over Wi-Fi using the LAN
test kit, which bypasses most of the product. The full path (browser →
Matchmaker → Host Agent → signed guest image → mesh → gateway) has still
never run on real machines; that needs the server (phase 3) and the signed
image (phase 4) first.

## Tests (2026-11-16, macOS)

`bash scripts/check.sh` runs all of these.

| Suite | Result |
| --- | --- |
| Host Agent, Rust (`cargo test --lib`) | 348 passed |
| Matchmaker (`pytest`, Postgres) | 255 passed |
| Web (`vitest`) + typecheck | 78 passed |
| Scripts and guest image (`pytest`) | 55 passed |
| Gateway (`pytest`) | 28 passed |
| Integration (`scripts/integration/run.py`) | 72 checks passed, twice in a row from an empty database |
| Lint (`ruff`) | clean |

No unit test starts a real VM, GPU bind, Headscale, or storage service.
Those are injected fakes.

The integration test starts the real Matchmaker and gateway as separate
processes on a real Postgres and drives: register → approve → book → poll
→ desktop through the gateway → end → credits, plus a failed boot and a
host going silent. The Host Agent, guest, Headscale, storage and
Cloudflare are stand-ins inside the test, so it proves the services agree
with each other, not that a VM boots. It is not yet confirmed in GitHub
Actions (nothing from this work is pushed).

## What has run on real hardware

| What | Status |
| --- | --- |
| LAN test kit, quick mode, Windows 10/11 PC (QEMU + WHPX) | Seen working 2026-11-14. Desktop opened from a Mac on the same Wi-Fi. |
| LAN test kit, quick mode, Arch Linux PC (QEMU + KVM) | Seen working 2026-11-14, after a manual firewall rule. |
| Changes made to the kit since then (virtio disk, VPN-safe address, firewall cleanup) | Not re-run on hardware. |
| Signed Packer guest image | Never built and booted on a host. |
| Host Agent provision path (Matchmaker tells the agent to boot a guest) | Never run on hardware. |
| Guest joining Headscale, mounting storage | Never run in a guest. Storage keys checked against the real R2 bucket; the desktop password checked against the real KasmVNC in a container. |
| Public gateway across two real networks | Never run. |
| Pro tier (GPU passthrough: Linux VFIO, Windows Hyper-V) | Never run. |
| Platform server (Matchmaker, Headscale, gateway, web) | **Deployed 2026-11-16 on Oracle Cloud** (Morocco West, Ubuntu 24.04, `VM.Standard.E5.Flex`: a paid shape running on trial credit, not Always Free). Storage is Cloudflare R2, checked from the server with per-user keys. **Public over HTTPS since 2026-11-17:** web page, `/api/docs` and mesh health answer, certificates issued, gateway joined the mesh. See `docs/test-logs/2026-11-16-oracle-deploy.md`. |

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

## Changed on 2026-11-16

1. **Server deployed** on Oracle Cloud with Cloudflare R2 (see the table
   above). Four deploy-script bugs fixed on the way (issue 22).
2. **R2 keys need no Cloudflare API token.** The Matchmaker signs each
   guest's key itself; confirmed against the real bucket. A wrong date
   format in the storage client, which real S3 rejects, was fixed (18).
3. **Desktop password per session.** Guests' desktops had no password on
   the mesh, so any host kit holder could open any renter's desktop. Now
   only the guest and the gateway know the password (issue 15).
4. **Host Agents fetch their commands** instead of being dialled through
   a shared SSH tunnel that did not exist on the new server (issue 19).
   The kits need no tunnel or port-forward any more.
5. **Guest fixes** for the next image build: files in `~/Persistent`
   belong to the desktop user, the join retries, `fusermount` link,
   plain paste in terminals (issues 20, 12).
6. The gateway forwards the WebSocket subprotocol the desktop client asks
   for, and dials the guest before accepting the browser.

Database migration `0010` adds the command queue, `hosts.pull_mode` and
the desktop password. **The guest image must be rebuilt** to pick up 3 and
5; an old image still works but its desktop has no password.

## Roadmap phases 1 and 2 (2026-11-15, in code)

None of this has run on real hardware or against real Cloudflare yet.

1. **Per-host credentials (issue 2).** Each Host Agent makes its own
   secret and sends it with every call; the Matchmaker refuses anyone else
   naming that host, and commands an agent with a token derived from that
   host's secret. The agent's listener accepts nothing else.
2. **Per-user storage keys on Cloudflare R2 (issue 1).** With
   `STORAGE_CREDENTIALS=r2` a guest gets a temporary key for one user's
   prefix in one shared bucket, read-only above the 20 GB quota.
   `scripts/vps-deploy.sh` takes five `DCL_R2_*` values and then skips
   renterd. Not done: encryption before upload (rclone crypt), and early
   revocation (R2 has none; keys expire after 12 hours).
3. **Renters are off the mesh (issue 15).** No `tag:user` key, no mesh key
   in session JSON, no `tag:user` rule in `headscale/acl.hujson`. The
   gateway is the only way to a desktop.
4. **Asynchronous booking (issue 4).** `POST /v1/sessions` returns 202 with
   `provisioning`; the browser polls `GET /v1/sessions/{id}`. New states
   `provisioning` and `failed`; new route `GET /v1/sessions/current`. The
   credit clock starts when the desktop is ready.
5. **Mesh keys are reused (issue 5).** One guest key per session.
6. **Web error states.** No host free, host failed to start, host went
   offline (with or without a move to another host), session ended. A
   "starting your computer" view with Cancel.
7. **Host Agent "Copy report".** Versions, capabilities, registration and
   the last provision error, with secrets blanked.
8. **Integration test** and a bug it found (issue 17): a failed boot was
   reported as ready when the host still had the link of an earlier
   session.

API changes other code must follow: session JSON has `status`,
`failureReason`, a nullable `connection`, and `mesh.guestHostname` instead
of `mesh.guest`/`mesh.user`. Database migration `0009` adds the host
secret columns, `sessions.failure_reason`, the wider occupancy indexes and
drops the unique-bucket rule. The signed guest image must be rebuilt: the
join script now exports `AWS_SESSION_TOKEN`.

## Changed earlier on 2026-11-15

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

**The order of work is now [`docs/roadmap.md`](docs/roadmap.md)** (phases 0–9
to a closed beta). It differs from the list below in one way: the first
end-to-end run (step 3 here) comes before direct peer-to-peer streaming
(step 2 here), which moves to after the beta. The sections below remain as
the detail for each item.

### 1. Fix the two security holes before anyone outside tests this

**Done in code 2026-11-15** (see "Roadmap phases 1 and 2" above and issues
1 and 2). Left: create the Cloudflare account, one R2 bucket, an R2 API
token and a Cloudflare API token that may create temporary credentials;
put the five `DCL_R2_*` values in `.env.vps`; then check on the real
account that a guest key is refused on another user's prefix. Encryption
before upload (rclone crypt) is not written yet.

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

**Done in code 2026-11-15.** How long the signed image takes to boot on
real hardware is still not measured; step 3 will show whether the 330 s
agent timeout and the 420 s `PROVISION_TIMEOUT_SECONDS` are enough.

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
