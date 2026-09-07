# Project Status — last updated: 2026-09-07, after phase 33 Host Agent binary

## Hardware (this Mac cannot host)

Never write “confirmed working” until a human reports a live session on metal.

- Standard-Linux: Implemented in code. Not yet run on physical hardware.
- Standard-Windows: Implemented in code. Not yet run on physical hardware.
- Pro-GPU: Implemented in code. Not yet run on physical hardware.

`SEED_HOST_KASM_URL` remains `https://example.invalid/kasm` for the seed
command. Successful provision no longer stores that placeholder. Pointers:
`docs/runbooks/gate-fc.md`, `gate-whpx.md`, `gate-pro.md`. Guest disks:
`guest-images/README.md`. There is no gate-result template.

## Done

- Specs: architecture + cahier in `docs/` (OS-conditional Standard Lab, GPU
  exclusivity, four planes). On `main`.
- Headscale operator runbook: `docs/runbooks/headscale.md` (ACL grants,
  session guest/user mint, platform exit-node guest path).
- renterd operator runbook: `docs/runbooks/renterd.md` (compose, S3 v4
  keypairs, guest rclone argv). **Implemented storage is renterd, not Storj
  (human decision 2026-09-05).**
- Execution phases 1–33 on local `main` (not pushed):
  - Host Agent: CPU/RAM via sysinfo; NVIDIA GPUs via nvml-wrapper; config v2
    (`sharedGpuId`); CSP, `freezePrototype`, capabilities off `core:default`;
    Rust Matchmaker client (register/PUT + 10s heartbeats + earnings GET).
    OS-conditional provisioner stubs + opt-in `POST /v1/provision`. Immediate
    pause writes `sharingEnabled` and skips heartbeats. Presence is paused /
    idle / in-session. Earnings copy is `{n} credits accrued (not paid out)`
    from the Matchmaker ledger (not a payout). Rust-side tray (tooltip +
    pause/resume; no JS tray APIs). **Phase 27:** Linux Standard HTTP
    provision starts the fetched Firecracker binary and PUTs boot-source,
    drives/rootfs, machine-config, then `InstanceStart` over a Unix socket
    (`firecracker.rs`). **Phase 28:** Windows Standard HTTP provision starts
    the fetched Weilnetz `qemu-system-*.exe` with qemu.org `-accel whpx`
    (`qemu.rs`; no TCG, no Hyper-V Standard boot). `plan()` still never
    invokes. After a successful Firecracker/QEMU invoke or VFIO/Hyper-V apply,
    provision polls for a reachable Kasm (`https://{meshHost}/`, port 443) or
    Sunshine (`https://{meshHost}:47990/`) locator and returns that URL, or
    400 if the streamer never answers. Tests inject loopback HTTP. Plan-only
    stubs may still return `.invalid`. **Phase 29:** Linux Pro HTTP
    provision applies vfio-pci against injectable sysfs (production: live
    `/sys`; `cfg(test)` refuses live `/sys`) with rollback. The window stays
    `apply: false`. After apply, provision polls Sunshine
    (`https://{meshHost}:47990/`) the same way. QEMU
    is not spawned with VFIO attached. **Phase 30:** Windows Pro HTTP
    provision applies Hyper-V DDA (preferred) or exclusive GPU-P through an
    injectable runner (production: `powershell.exe -NoProfile
    -NonInteractive`, 30s kill; `cfg(test)` refuses live PowerShell) with
    rollback. The window stays `apply: false` / `hyperv: None`. After apply,
    provision polls Sunshine the same way (honest 400 if `dcl-pro-guest` is
    not serving). `hypervisorInvoked` stays false on the Hyper-V apply path.
    No `Start-VM`. VM `dcl-pro-guest` must already exist and be powered off.
    Missing kernel/binary/rootfs/WHPX/VFIO/Hyper-V
    mid-step is 400, not a fake success. macOS/Linux/Pro do not spawn QEMU; macOS/
    Windows/Pro do not spawn Firecracker. Tests inject a fake process (no
    `/dev/kvm`, no WHPX) and a mock PCI tree (no live `/sys`) and a fake
    Hyper-V runner (no `powershell.exe`). **Before** a stub
    boot, `image.rs` verifies Ed25519 over the SHA-256 digest against
    `IMAGE_SIGNING_PUBLIC_KEY` and `IMAGE_LOCAL_DIGEST` (required when the
    provision listener is on). After register, if `HEADSCALE_LOGIN_SERVER` is
    set, `mesh.rs` POSTs `/v1/hosts/{id}/mesh/preauth` from Rust and runs
    `tailscale up` with that env URL + the minted key (never
    `--advertise-exit-node`, `--exit-node`, or `--advertise-tags`). Provision
    may include storage locators (endpoint/bucket/prefix/region, no keys) and
    a guest-mesh hint (no preauth). The stub never rclone-mounts on the host
    OS and never `tailscale up`s as the guest. When locators are complete it
    returns `guestRclone` argv (`--s3-env-auth`). Pro HTTP 200 adds `streamer:
    sunshine`, stub `sunshineUrl`, and a Moonlight host hint (`gpuExclusive`
    true). Standard stays `streamer: kasm`. **Phase 18:** `host_capabilities.rs`
    probes OS, Linux `/dev/kvm` r/w (path-injectable), Windows WHPX/Hyper-V as
    a stub enum on non-Windows, GPU list + second-display heuristic (iGPU or
    2+ GPUs), and Linux IOMMU sysfs (unknown off Linux). The window shows
    Standard/Pro **offered** in plain language and never asks which backend.
    **Phase 19:** `hypervisor.rs` locates or fetches Firecracker on Linux
    Standard (documented GitHub v1.16.1 tarball + published SHA-256) into a
    user cache; checksum mismatch fails fast and does not install; this
    module never executes the binary. Windows Standard fetches the pinned
    Weilnetz NSIS installer (`qemu-w64-setup-20260811.exe` / ARM sibling),
    verifies the published SHA-512, and runs `/S` `/D=<user cache>` (phase 24
    / leftover **19b**). UAC consent is still manual (`qemu.nsi`
    RequestExecutionLevel admin). macOS skips with a logged reason. Tests
    inject a fetcher / installer / `file://` / loopback httptest (no live
    GitHub or Weilnetz in CI).     **Phase 20:**
    `image.rs` locates or fetches the signed guest image for the probed
    backend into a user cache. Linux Standard fetches the Firecracker guest
    image; Linux Pro (when offered) fetches the Pro image. Windows Standard
    fetches the QEMU disk the same way (phase 28). SHA-256 + Ed25519
    (`IMAGE_SIGNING_PUBLIC_KEY` + Matchmaker `IMAGE_*` digest/signature) run
    **before** the file is treated as ready. Mismatch fails fast and does not
    leave a ready disk. macOS skips with a logged reason. Tests inject a
    fetcher / `file://` / httptest (no multi-GB download). The webview does
    not fetch.
    **Phase 21:** `launch.rs` maps clamped v2 sliders to a launch-config JSON
    for the stub backend (`firecracker` / `qemu-whpx` / `qemu-vfio` /
    `hyperv-gpu`). vCPU and memory come from the sliders. Standard omits GPU
    assignment. Pro includes `{ id, exclusive: true }` when `shareGpu` is on.
    Kernel/rootfs paths come from the phase-20 cache when present. Below
    2 vCPU / 4 GiB is a plain-language error (phase 23 owns full UX). macOS
    is still unsupported. The JSON document still has
    `hypervisorInvoked: false`; live invoke is `firecracker.rs` / `qemu.rs` on
    the HTTP port. Linux Pro VFIO apply and Windows Pro Hyper-V apply are
    `gpu_bind.rs` on the HTTP port (not the window). **Phase 22:**
    `gpu_bind.rs` plans exclusive Pro GPU assignment. Linux is vfio-pci
    (unbind host driver → bind vfio-pci) with injectable sysfs/driver paths
    and rollback to the previous driver on apply failure. PCI addresses come
    from sysfs (not `lspci`). Missing IOMMU or a second display path refuses
    before any write. Windows prefers Hyper-V DDA (`Disable-PnpDevice`,
    `Dismount-VMHostAssignableDevice -Force`, `Add-VMAssignableDevice`) and
    falls back to exclusive GPU-P (`Set-VMHostPartitionableGpu -PartitionCount
    1`, `Add-VMGpuPartitionAdapter`, `Set-VMGpuPartitionAdapter` at
    100000000). Apply uses an injectable runner; production is
    `powershell.exe -NoProfile -NonInteractive` with a 30s kill. Tests inject
    a fake runner and never spawn PowerShell. Mid-step failure rolls back
    (Mount + Enable-PnpDevice, or Remove-VMGpuPartitionAdapter). The window
    is plan-only (`apply: false`). **Phase 29** applies Linux VFIO from
    Matchmaker provision (`apply: true` on that path only). **Phase 30**
    applies Windows Hyper-V DDA/GPU-P from Matchmaker provision (`apply:
    true` on that path only). Firecracker is never used for Pro. GPU
    exclusivity copy is unchanged. Tests use a mock sysfs tree / fake Hyper-V
    runner (no root, no real GPU). **Phase 23:** `eligibility.rs` combines the
    probe, the
    2 vCPU / 4 GiB / 50 GiB floor, and a refused VFIO/DDA plan into one
    Standard/Pro offer. The window shows **Available** vs **Not available**
    with the reason. A blocked tier is not offered (no launch document; live
    `POST /v1/provision` is 400 with plain-language `detail`, no stub URL,
    occupancy unset). GPU exclusivity copy stays when Pro *is* offered.
    Unknown spare disk does not block. **Phase 24:** Windows QEMU silent
    install (leftover **19b**): detect missing QEMU, fetch the documented
    NSIS installer, checksum, run `/S` `/D=`. Injected installer in tests.
    UAC is not automated. **Phase 25:** leftover **22b** Hyper-V DDA/GPU-P
    apply + rollback (injectable runner). **Phase 26:** Packer + QEMU guest
    image pipeline in `guest-images/` (Standard = Kasm Workspaces in the
    guest; Pro = Sunshine in the guest). Operator build emits `rootfs.img` /
    `rootfs.qcow2` plus `.sha256` / `.sig` / `.manifest.json` using the same
    Ed25519-over-raw-SHA-256 contract Host Agent already verifies. Dev
    private key from `IMAGE_SIGNING_PRIVATE_KEY` only; never committed.
    **266/266** `cargo test --lib` (counts in the phase-27 handoff).
    **Phase 28:** Windows QEMU `-accel whpx` + Windows Standard image
    fetch/verify. **Phase 29:** Linux Pro VFIO bind apply from provision.
    **Phase 30:** Windows Pro Hyper-V DDA/GPU-P apply from provision.
    **Phase 31:** live Kasm/Sunshine locators after invoke/apply (loopback
    injected in tests). **Phase 33:** on-target `npm run tauri build` (Linux
    AppImage/deb, Windows MSI/NSIS) documented; packaged `.env` from cwd /
    exe dir / app-config (or `HOST_AGENT_ENV_FILE`); `0.0.0.0` is not
    advertised as `agentUrl` (set `HOST_AGENT_URL`); optional GitHub Actions
    workflow builds on Linux/Windows runners (artifacts, not git). This Mac
    cannot emit those installers. **328/328** `cargo test --lib` (this phase).
  - Matchmaker: Argon2id + JWT-from-env; signup/login/me; host inventory
    pending → manual approve; heartbeats; occupancy partial unique indexes
    `sessions_one_active_per_host` / `sessions_one_active_per_user`;
    `HostSelector.select(db, *, lab_tier) -> Host | None` via
    `get_host_selector()` → `RankedHostSelector`. `HostProvisioner` after
    occupancy commit, now advertising `imageDigest` + `imageSignature` for the
    booked lab tier plus optional storage locators and a guest-mesh hint (no
    preauth, no S3 keys). `GET /v1/images/{labTier}` is checksum metadata
    only. `POST /v1/hosts/{id}/mesh/preauth` mints a Headscale preauth key
    tagged **`tag:host`**. `POST /v1/sessions` mints ephemeral **`tag:guest`**
    and **`tag:user`** keys (Headscale 0.26 `aclTags`) with unique hostnames
    and puts `--exit-node=$HEADSCALE_EXIT_NODE` on the guest join plan
    (default `dcl-platform-exit`). Session end / silence interrupt best-effort
    expires those hostnames. `POST`/`GET /v1/storage` and session JSON
    `storage` issue a per-user renterd S3 bucket (`dcl-<uuid>`) and guest
    rclone locators + `rcloneArgs` (**no access keys** on the wire;
    `--s3-env-auth`). `GET` is 404 until issued. Bucket provision on book runs
    **before** occupy; renterd or Headscale mint failure is 502 without taking
    the host. Session `connection` is Kasm for Standard and Sunshine/Moonlight
    for Pro (`gpuExclusive: true`) — not Kasm renamed. Locator still lives in
    `hosts.kasm_url`. Session end meters `startedAt` → `endedAt` into
    `ledger_entries`. Silence (~45s) on an occupied host marks the session
    `interrupted`, settles the ledger once, expires session mesh nodes, and
    rematches via one `select` (same renterd bucket follows the user; rematch
    does not call renterd). Required env includes `CREDITS_PER_MINUTE`, the
    four `IMAGE_*` pairs, `HEADSCALE_*` (optional `HEADSCALE_EXIT_NODE`,
    default `dcl-platform-exit`), and `S3_ENDPOINT` / `S3_ACCESS_KEY` /
    `S3_SECRET_KEY` (or `RENTERD_S3_*` aliases).     **Phase 31:**
    `HttpHostProvisioner` stores a live locator even when
    `hypervisorInvoked` is true, rejects `.invalid`, and the session router
    deletes the occupancy row (502) if provision fails while the host row is
    still a placeholder. **166/166** pytest. Session JSON still
    has no `intent` field. No latency/reliability columns. No Matchmaker
    pause field. No payment processor. No image CDN / multi-GB download. No
    live Sia network in unit tests. No live Headscale in unit tests.
  - Web harness: signup/login, FR2 intent prompt → `labTier`, automatic
    `POST /v1/sessions` (no host picker; FR3 `HostSelector` on Matchmaker).
    Phase-31 live locators (Kasm mesh `/`, Sunshine `:47990/`) render a
    clickable Open desktop / Open Sunshine web UI link; `.invalid` stays
    hidden (no iframe). Pro shows Moonlight host/app when that host is not a
    placeholder. Login restores an `active` row from `GET /v1/sessions`.
    End/history/credits come from Matchmaker (`POST .../end`, `GET /v1/sessions`).
    Optional storage locator (endpoint/bucket only — not secrets), mesh
    hostname/tags/exit-node (not the preauth secret). GPU exclusivity copy
    stays. **52/52** vitest; `npm run typecheck` passed.
  - Headscale compose in `headscale/` (image `0.26.1`) with checked-in
    default-deny `acl.hujson`. Guest-path exit-node overlay
    `docker-compose.exit-node.yml` (`TS_HOSTNAME: dcl-platform-exit`). renterd
    compose in `renterd/` (image `ghcr.io/siafoundation/renterd:2.9.3`, zen
    testnet, S3 on loopback **8081**). No Tailscale cloud account. No Storj.
  - Guest image **build** (phase 26): `guest-images/` Packer qemu `disk_image`
    from the pinned Ubuntu 24.04 cloud image (not in git). Standard provisioner
    runs Kasm `install.sh --accept-eula --admin-password --user-password`.
    Pro provisioner `dpkg -i` the official Sunshine Ubuntu 24.04 `.deb` and
    enables `app-dev.lizardbyte.app.Sunshine`. `scripts/sign_image.py` writes
    checksum + Ed25519 sidecars. Firecracker `vmlinux` is an extra operator
    input (`FIRECRACKER_KERNEL`); Packer does not invent one.

## In Progress

- None. Numbered software phases 1–33 are on local `main`. Hardware is
  implemented in code, not yet run on physical hardware.

## Up Next

Leftover paper (not a numbered software phase). Operator path:
`docs/phase-logs/phase-33-host-agent-binary.md` and `host-agent/README.md`
(build on Linux/Windows, env, Packer images, kernel, UAC, Hyper-V VM name).
Do not switch matching to manual host browsing.

Leftover paper:

1. Pairwise mesh ACLs — Headscale 0.26 file policy cannot mint
   `tag:session-<uuid>` owners; every `tag:user` may still reach every
   `tag:guest`.
2. Live guest Tailscale/rclone — join/mount plans exist; the Packer image
   installs the clients; they still need a booted guest with a network.
3. FR13 latency ranking waits on a measured value — do not fake `latency_ms`.
4. TAP/net so Kasm/Sunshine is reachable on metal, jailer, QEMU VFIO attach
   after bind. The Host Agent window stays plan-only for GPU bind.

## Known Risks / Open Items

- **Standard-Linux / Standard-Windows / Pro-GPU: implemented in code, not
  yet run on physical hardware.** No live session URL in this repo. This Mac
  still cannot host. `SEED_HOST_KASM_URL` remains `https://example.invalid/kasm`
  for the seed command. Successful provision overwrites the host locator
  with a reachable URL or fails the book. Do not invent lab output. Never
  write “confirmed working” without a human metal report.
- **Phase 15 race (2026-09-05):** `0a59f68` issued renterd S3 keys on session
  JSON; `9f69179` kept keys in env and locators-only on
  `POST`/`GET /v1/storage`. Phase 16–17 kept the locators-only contract.
  An unused `0007_host_latency_ms` draft from a parallel agent was **not**
  applied.
- Local `main` is ahead of `origin/main` (planning + phases 1–33). Nothing was
  pushed this session.
- Host register/heartbeat/provision/earnings/mesh-preauth auth uses a shared
  `HOST_BOOTSTRAP_TOKEN`. Per-host tokens are not in this MVP.
- Existing local `matchmaker/.env` must have `HOST_BOOTSTRAP_TOKEN` and
  `HOST_APPROVE_TOKEN` (different values), `CREDITS_PER_MINUTE`, the four
  `IMAGE_*` digest/signature pairs, `HEADSCALE_*`, and `S3_*` (or
  `RENTERD_S3_*` aliases) or uvicorn fails at startup. `HEADSCALE_EXIT_NODE`
  defaults to `dcl-platform-exit` if unset (`off` disables guest `--exit-node`).
  Pytest overwrites Headscale and renterd env to `.invalid` so a local `.env`
  cannot point tests at a live instance. Do not commit `.env`, renterd seeds,
  or `renterd/data/`. Copy fixture `IMAGE_*` values from `.env.example` for
  local stub verify; they are not a production key. After a real Packer
  build, replace them from `guest-images/output/<tier>/env.snippet`. Optional
  `STALE_SESSION_SWEEP_INTERVAL_SECONDS` defaults to 15.
- Enabling `HOST_AGENT_LISTEN_ADDR` now also requires `IMAGE_SIGNING_PUBLIC_KEY`
  and `IMAGE_LOCAL_DIGEST` (fail fast). Linux Standard fetch also needs
  `IMAGE_STANDARD_SIGNATURE` and `HOST_AGENT_STANDARD_IMAGE_URL` (checksum
  metadata is the same Matchmaker `IMAGE_*` pairs). Public key only — never a
  signing secret in the agent or Matchmaker runtime. Linux Standard invoke
  also needs `FIRECRACKER_KERNEL` (or `vmlinux` next to the verified rootfs).
  Windows Standard invoke also needs WHPX in the probe, Weilnetz QEMU
  installed, and a verified Standard disk (`HOST_AGENT_STANDARD_IMAGE_URL`).
- A long-running Matchmaker on `:8000` from 2026-09-02 still has no
  `GET /v1/sessions` (405) and no `streamer` / `mesh` fields. That is a
  **stale uvicorn**, not a web/API method mismatch (`POST /v1/sessions`
  books; `GET /v1/sessions` is history). Restart Matchmaker after pulling
  this commit, with `HEADSCALE_*`, `IMAGE_*`, and `S3_*` (or `RENTERD_S3_*`)
  set. Do not migrate under that process from a web-only session without
  replacing it.
- Pause is still local `sharingEnabled=false` (no Matchmaker field). Skipping
  heartbeats looks like disconnect: after 45s the occupied session is
  interrupted and may rematch.
- Local Host Agent occupancy (in-session) is set on provision HTTP 200 (live
  locator returned) and is
  **not** cleared when Matchmaker interrupts — there is no teardown HTTP port
  on the agent. Timeout 400 does not occupy. Resume after pause can still show in-session if a guest already
  left. A successful Firecracker InstanceStart or QEMU `-accel whpx` spawn
  also leaves the child detached with no teardown port. A successful Linux Pro
  VFIO bind leaves the GPU on vfio-pci with no unbind teardown port. A
  successful Windows Pro Hyper-V DDA/GPU-P assign leaves the GPU on the
  guest VM with no unassign teardown port.
-   Linux Standard HTTP provision can InstanceStart. Windows Standard HTTP
  provision can spawn QEMU `-accel whpx`. Linux Pro HTTP provision can apply
  vfio-pci. Windows Pro HTTP provision can apply Hyper-V DDA/GPU-P. `plan()`
  stays a stub.
  Launch-config JSON still has `hypervisorInvoked: false`. GPU bind: window
  is plan-only; Linux Pro provision apply uses injectable/live sysfs with
  rollback (tests refuse live `/sys`). Windows Pro provision apply uses an
  injected Hyper-V runner (production PowerShell; tests refuse live
  `powershell.exe`). Image fetch+verify is the software contract (small fixture
  bytes in tests), including Windows Standard. Combined eligibility (phase 23)
  refuses a blocked tier in the window and on the live provision listener. On
  macOS, a real Host Agent `POST /v1/provision` is 400 (unsupported OS / not
  offered) even with a valid signature; Matchmaker then fails the book (502)
  if the host row is still a placeholder seed. Guest-image ensure, launch-config
  generation, and GPU bind on this Mac skip / fail as unsupported. Packer build
  needs Linux/KVM (or HVF) and is not run in CI.
  Metal Firecracker still needs KVM, a real `rootfs.img`, operator `vmlinux`,
  and TAP/net — InstanceStart plus locator poll still needs a reachable Kasm.
  Metal QEMU+WHPX still needs WHPX, UAC for NSIS, a verified disk, and guest
  net. Metal VFIO still needs IOMMU + a second display path, then QEMU attach.
  Metal Hyper-V still needs the feature, elevation, and `dcl-pro-guest` —
  DDA/GPU-P assign does not `Start-VM`; Sunshine poll fails honestly if the
  guest is not serving.
- Intent text stays in the harness. It is not persisted and is not on the
  session JSON. Keyword mapping is a smallest-slice heuristic (office/code →
  Standard; GPU/game/3D/training → Pro).
- Ranking uses declared caps on the host row. There is no measured latency or
  reliability score. A concurrent race on the same top-ranked host is still
  201+409 (no re-select of the runner-up on IntegrityError).
- Rematch is one `HostSelector.select`. If none is free, the user has no
  active session; the next book is 409 until a fresh approved host exists.
  Reclaim runs before a new book, so a displaced user can take the last
  remaining live host. The renterd bucket is per-user and is not re-created
  on rematch. Rematch mints a new guest/user mesh for the new session id.
- **Real host payouts and user card charges are deferred.** The ledger records
  credits only. Do not add Stripe/PayPal.
- **Tailscale SaaS is not used.** Self-hosted Headscale mint+join is in.
  Default-deny ACLs are `headscale/acl.hujson`. Host mint is `tag:host`.
  Session mint is ephemeral `tag:guest` + `tag:user` with unique hostnames.
  ACL grants stay role-based (not pairwise).
- **Architecture §9 item 5 — software-applied on the guest join plan, not
  live-verified.** Guest `tailscaleUp` includes `--exit-node` from
  `HEADSCALE_EXIT_NODE` (default overlay hostname `dcl-platform-exit`). Host
  Agent join never advertises `--advertise-exit-node` and never sets
  `--exit-node`. Until a guest image actually runs those args against a
  running exit node, guest internet can still egress via the host path.
- **FR4/FR6 live mount is still paper.** Control-plane grant + argv/plan +
  provision `guestRclone` exist. The Packer image installs rclone. rclone
  inside a booted guest is still later. Session keys must not land on the
  volunteer host disk.
- Consumer GPU EULAs and ToS/liability still need legal review (architecture
  §9 items 10–11).
- Aikido scan: MCP `namespaceStatus: error` this session (`mcp_auth` timed
  out; tool discovery failed). Not a scan pass. Run `/aikido:setup`.

## Notes for next agent/session

- Execution **phase 33 is done** (on-target Host Agent build + packaged
  `.env` + fail-fast `HOST_AGENT_URL` when listen is `0.0.0.0`). Hardware
  lines above are **not** “confirmed working”. No further numbered software
  phase is required to copy the agent onto a host PC. Leftover is metal
  (TAP/net, Packer disk, kernel, UAC, Hyper-V VM, human confirmation).
  Do not start Storj. Do not invent lab output, a live metal session, or `latency_ms`. Do not treat renterd compose-up
  as a live desktop. Do not weaken HostSelector, occupancy indexes, GPU
  exclusivity copy, OS-conditional backends, or CSP. Do not switch matching
  to a host catalog unless a later product decision says so — flag it, do
  not silently implement. Do not fetch Firecracker,
  QEMU, or a guest disk from `cargo test` (injected fetcher / installer /
  file:// / httptest). Do not spawn a real Firecracker or QEMU from tests
  (inject `FirecrackerRuntime` / `QemuRuntime`). Do not use Firecracker for
  Pro. Do not bind live `/sys` from tests. Do not spawn live PowerShell from
  tests (inject `HyperVGpuOps`). Do not commit Packer output, PEMs, or Kasm
  passwords. Do not `Start-VM` as a Standard boot fallback or as a Hyper-V
  attach step. Do not surprise-bind
  from the settings window.
- Session router uses `HostSelector` + `HostProvisioner` via Depends. Do not
  import `RankedHostSelector` in `sessions.py` or `silence.py`. Do not put
  Firecracker/QEMU/WHPX names there. Occupancy stays on session indexes.
  Matchmaker stores a provision result only when the locator is not
  `.invalid`. Invoke plus a live URL is kept. Placeholder provision fails
  the book (502) instead of occupying the host with the seed.
- Storage: Matchmaker talks path-style S3 to renterd (`HttpRenterdClient`).
  Tests swap `get_renterd_client()`. Never point pytest at a live Sia
  network. Guest rclone uses `--s3-env-auth` so secrets stay out of argv.
  No access keys on `/v1/storage` or session JSON.
- Image verify: signed message is the raw 32-byte SHA-256 digest; signature is
  standard base64 Ed25519; public key is 64 hex chars. Matchmaker does not
  hold the private key. Tests use a documented fixture seed in Rust only.
  Phase 20 fetch reuses those `IMAGE_*` checksums; bytes come from
  `HOST_AGENT_*_IMAGE_URL` (injected in tests). Phase 26 `sign_image.py`
  emits the same shape for a real disk.
- Launch config: `host_agent_lib::launch::build` takes OS + lab tier + clamped
  `SharingConfig` and optional probe + spare disk. JSON fields are `backend`,
  `labTier`, `vcpuCount`, `memoryMib`, `kernelPath`, `rootfsPath`, optional
  `gpu: { id, exclusive }`, `hypervisorInvoked: false`. Standard omits `gpu`.
  Below 2 vCPU / 4 GiB / 50 GiB disk, or a probe that does not offer the
  tier, is `Err` (`TierNotOffered` / resource errors). Combined offers live
  in `eligibility.rs`. Live Firecracker invoke is `firecracker::boot_from_launch`.
  Live QEMU+WHPX invoke is `qemu::boot_from_launch`.
- GPU bind: `host_agent_lib::gpu_bind::assign` / `report` take injectable
  `PciSysfs` paths and optional `HyperVGpuOps`.   Linux Pro plans vfio-pci and
  applies from Matchmaker provision (`apply: true`) against injectable sysfs
  (production live `/sys`; tests refuse live `/sys`) with rollback. Windows
  Pro plans DDA from the window; apply prefers DDA then exclusive GPU-P
  through the runner, with rollback, from Matchmaker provision (`apply:
  true` when `ProvisionGate.hyperv` is set). UI command is
  plan-only (`WINDOW_GPU_BIND_APPLY` / `apply: false` / `hyperv: None`). Never Firecracker. A
  refused plan with shareGpu on blocks the Pro offer. Live Linux Pro VFIO
  apply is `provision_http` when `ProvisionGate.vfio` is set. Live Windows
  Pro Hyper-V apply is `provision_http` when `ProvisionGate.hyperv` is set.
- Mesh: Matchmaker holds `HEADSCALE_API_KEY`; the agent never does. Host join
  uses `HEADSCALE_LOGIN_SERVER` from agent env plus a minted preauth key tagged
  `tag:host`. Session JSON `mesh` holds guest/user keys. Unit tests mock
  Headscale HTTP. Headscale 0.26.1 wants a numeric `HEADSCALE_USER`. Policy
  expects the Headscale user named `dcl` (`dcl@`).
- Metering is `ledger.settle_ended_session` at user end and on silence
  interrupt. Unique `(session_id, party)` prevents double-credit. Rate is
  `CREDITS_PER_MINUTE`.
- Stale threshold is **45 seconds**. Agent ping interval is **10 seconds**.
  Never-seen hosts are not selectable. Heartbeat stamps `lastSeenAt` **before**
  reclaim so a late ping keeps that host's own session.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.”
