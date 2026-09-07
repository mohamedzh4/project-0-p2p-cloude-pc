# Handoff — phase 26 guest image pipeline (2026-09-07)

- Phase goal: Packer pipeline that emits signed Standard (Kasm) and Pro
  (Sunshine) guest disks Host Agent can fetch; drop gate-template paperwork;
  inventory remaining stubs. No Firecracker/WHPX boot, no live VFIO, no web loop.
- Verified git state at start: `8ededf8` on local `main` (phase 25). Did not
  rewrite `a2be066` / `8ededf8`. Did not push. Did not force-push. Did not
  touch `pr/*`.

## Stub / mock / log-only inventory

Today vs “real”. None of this is confirmed working on physical hardware.

- **Firecracker invoke / microVM boot — stub.**
  `host-agent/src-tauri/src/provision.rs` `FirecrackerStub::plan` /
  `plan_provision` return `https://provision.invalid/firecracker` with
  `hypervisorInvoked: false`. `hypervisor.rs::ensure` locates/fetches the
  binary (SHA-256) and never executes it. `launch.rs::build` writes JSON
  (`hypervisorInvoked: false`). No `Command::new("firecracker")`. Real =
  `InstanceStart` with kernel + this phase’s `rootfs.img`.
- **QEMU WHPX / Hyper-V fallback boot — stub.**
  `provision.rs` `QemuWhpxStub::plan` → `https://provision.invalid/qemu-whpx`.
  `hypervisor.rs::locate_or_install_qemu` fetches the Weilnetz NSIS installer
  and runs `/S /D=` (install only). No `qemu-system-*.exe -accel whpx`.
  Hyper-V as a Standard fallback boot path is not implemented (Hyper-V is the
  Pro GPU backend). Real = boot the Packer qcow2 with WHPX (not TCG).
- **VFIO bind (Linux) — plan vs injectable sysfs.**
  `gpu_bind.rs::assign` can write bind/unbind/new_id (`SysfsBindOps`). The
  window (`commands.rs::plan_gpu_bind`) always passes `apply: false` (plan
  only). Tests use a mock tree; live `/sys` is refused under `cfg(test)`.
  Real = apply on metal when Pro is offered, with rollback, host display
  still on the other GPU.
- **Hyper-V DDA/GPU-P — injectable vs production PowerShell.**
  `gpu_bind.rs` production runner is `powershell.exe -NoProfile
  -NonInteractive` (30s kill) with documented DDA then exclusive GPU-P
  cmdlets. Window is plan-only (`apply: false`, `hyperv: None`). Tests inject
  a fake runner. Does not `Start-VM`; needs existing `dcl-pro-guest`. Real =
  elevated agent on a Hyper-V host actually assigns the GPU.
- **Guest image build — this phase (pipeline in-repo, not yet run on metal).**
  `guest-images/scripts/build.sh` + Packer qemu `disk_image` + Kasm/Sunshine
  provisioners + `sign_image.py`. CI does not run Packer. Real = operator
  runs the build and hosts `rootfs.img`.
- **Guest image fetch/verify — real-ish (phase 20), fixtures in tests.**
  Linux `image.rs::ensure_linux` / `locate_or_fetch_image`: download, SHA-256,
  Ed25519 over the raw 32-byte digest, fail-fast, no leftover disk.
  Windows `ensure_windows`: log **would fetch**, no download. macOS skip.
  Tests: injected fetcher / `file://` / httptest, bytes
  `dcl-fixture-standard-v1\n`. Matchmaker `GET /v1/images/{labTier}` is
  checksum metadata only (`images.py` / `routers/images.py`), not a CDN.
- **`SEED_HOST_KASM_URL` / locators — placeholder.**
  `matchmaker/.env.example` and `seed.py` use
  `https://example.invalid/kasm`. `provision_http.rs` returns
  `https://provision.invalid/{backend}` and Pro
  `https://provision.invalid/sunshine` + Moonlight hint
  `provision.invalid`. `HttpHostProvisioner` keeps the seeded host-row
  locator if the agent is down or claims `hypervisorInvoked: true`.
  Session JSON still stores locators in `hosts.kasm_url`.
- **Web session link / matching — harness placeholders.**
  `web/src/connection.ts` `isLiveConnectionUrl` hides RFC 2606 `.invalid`.
  `SessionPanel` will not “Open desktop” on the seed/stub URLs. Matching is
  real `RankedHostSelector` (declared CPU/RAM, Pro GPU id) — not latency.
  No iframe. Not the product dashboard.
- **Also stub / plan-only (for completeness):** `storage.rs` rclone argv,
  never spawned on the host; Matchmaker guest mesh keys are real mints
  against mocked Headscale in tests; Host Agent `mesh.rs` *does* run
  `tailscale up` for the volunteer (tag:host) when env is set — that path
  is not a guest boot.

## Files created/changed

- `guest-images/` (Packer HCL, cloud-init, provision scripts, signer, tests,
  README)
- `docs/runbooks/gate-fc.md`, `gate-whpx.md`, `gate-pro.md` reduced to
  pointers at `PROJECT_STATUS.md` (old PASS/result-template process removed)
- `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`, architecture
  snippet, `.gitignore`, Host Agent / Matchmaker `.env.example`, READMEs,
  headscale/renterd runbook intros
- `docs/phase-logs/phase-26-guest-image-pipeline.md` (this file)

## Key decisions

- Cloud image + Packer `disk_image`, not a committed ISO. Operator may set
  `UBUNTU_CLOUDIMG` to a local file.
- Kasm flags from current single-server docs: `install.sh --accept-eula
  --admin-password --user-password --swap-size 4096`, tarball 1.19.0 +
  published SHA-256.
- Sunshine: official `dpkg -i sunshine-ubuntu-24.04-{arch}.deb` and
  `systemctl --user --now enable app-dev.lizardbyte.app.Sunshine`.
- Signing: same as `image.rs` (Ed25519 over raw 32-byte SHA-256). Dev PEM
  from `IMAGE_SIGNING_PRIVATE_KEY`. Sidecars next to the image.
- Firecracker kernel is a separate operator input (`FIRECRACKER_KERNEL`).
  Packer will not guess a vmlinux from GRUB.
- Host Agent still does not boot.

## Test results

- Guest-image unittest: **12 passed**, 0 failed (`python3 -m unittest discover -s guest-images/tests`). Templates (Kasm vs Sunshine), checksum/signature sidecars, env snippet, refuse missing key, extract-kernel does not guess.
- Host Agent: **246 passed**, 0 failed (`cargo test --lib` in `src-tauri`). Unchanged.
- Matchmaker: **161 passed**, 0 failed (`matchmaker/.venv/bin/pytest`). Unchanged (`.env.example` comment only).
- Web: not run (README only).

## Flagged for the next agent

- **Next phase: Firecracker real invoke** using a phase-26 disk + phase-19
  binary + phase-21 launch config. Not WHPX boot, not live VFIO from the
  window, not the web loop.
- Hardware lines in `PROJECT_STATUS.md` are **not** “confirmed working”.
- Aikido MCP `namespaceStatus: error`; not a scan pass. Run `/aikido:setup`.
- Granola MCP needed auth; skipped.
- `origin/main` still behind. Nothing pushed.
