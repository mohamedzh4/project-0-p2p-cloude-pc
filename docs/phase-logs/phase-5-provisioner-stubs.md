# Handoff — phase 5 provisioner stubs (2026-09-04)

- Phase goal: OS-conditional provisioner contract in Host Agent Rust (stubs
  only) so Linux/Windows/Pro are peer backends before any real hypervisor call.
- Verified git state at start: `c46af88` on `main`, working tree clean, branch
  ahead of `origin/main` by 5. Did not push.
- Files created/changed:
  - `host-agent/src-tauri/src/provision.rs` (new: `HostOs`, `LabTier`,
    `ComputeBackend`, `Provisioner` trait, peer stubs, `plan`)
  - `host-agent/src-tauri/src/lib.rs` (module + `plan_provision` re-export)
  - `host-agent/README.md`
  - `matchmaker/src/matchmaker/routers/sessions.py` (comment only; selector and
    occupancy unchanged)
  - `matchmaker/README.md` (points phase 6 at `provision::plan`)
  - `PHASE_PLAN.md` (stable contract + phase 6 pointer)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/phase-logs/phase-5-provisioner-stubs.md` (this file)
- Key decisions made and why:
  - Selection is `match (HostOs, LabTier)` into a peer enum
    (`Firecracker` / `QemuWhpx` / `QemuVfio` / `HyperVGpu`), not a Linux
    Firecracker path with a Windows `if`. Each stub implements `Provisioner`.
  - `HostOs` is passed in (tests cover Linux/Windows on this Mac).
    `HostOs::current()` uses `cfg(target_os)`. macOS/`Other` return
    `UnsupportedHostOs` and do **not** alias to Firecracker.
  - Pro is a distinct target: Linux → QEMU+VFIO, Windows → Hyper-V GPU
    assignment. `plan` refuses without a non-blank `sharedGpuId`.
    `gpu_exclusive: true` plus `exclusivity_warning` (host loses that GPU).
    Standard ignores a GPU id and stays a CPU/RAM slice.
  - No Tauri types in selection. No HTTP port, no session-router call (phase
    6). `hypervisor_invoked` is always false. Stub URL origin
    `https://provision.invalid/{backend-slug}`.
  - `HostSelector.select` signature and occupancy indexes untouched.
- Test results:
  - Host Agent: **66 passed**, 0 failed (`cargo test`). Was 52. Added 14
    provisioner tests (Linux→Firecracker, Windows→QEMU+WHPX, peer Standard
    backends, Linux Pro≠Firecracker, Windows Pro≠QEMU+WHPX, Pro requires GPU
    id, blank id refused, Standard ignores GPU id, macOS unsupported, no
    hypervisor invoke, `plan` == select+plan). `cargo clippy -- -D warnings`
    clean. `npm run typecheck` clean.
  - Matchmaker: **52 passed**, 0 failed (`pytest`). Unchanged behavior
    (comment only). Concurrent booking still 201+409.
  - Web suite not re-run (unchanged this session).
- Contract types for phase 6 / orchestrator:
  - Call `host_agent_lib::plan_provision(os, &request)` (`provision::plan`).
  - `ProvisionRequest { lab_tier: LabTier, shared_gpu_id: Option<String> }`
    (`LabTier::parse("standard"|"pro")`).
  - `ProvisionPlan { backend, lab_tier, gpu_exclusive, shared_gpu_id,
    exclusivity_warning, connection_url, hypervisor_invoked }`.
  - Errors: `UnsupportedHostOs`, `GpuIdRequired`, `UnknownLabTier`,
    `TierBackendMismatch`.
  - Do not put Firecracker/QEMU/WHPX names in `sessions.py`. Do not invent a
    second OS selector in Python. Do not invoke a real hypervisor.
- Flagged for the orchestrator:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** Runbooks untouched; no
    `gate-*-result.md`; seed URL still `https://example.invalid/kasm`.
  - Matchmaker still does not call the Host Agent. Session booking still uses
    the Kasm URL on the host row.
  - Aikido MCP `namespaceStatus: error` / no usable `aikido_full_scan`; not a
    scan pass.
  - `origin/main` still behind local `main`. Nothing pushed.
  - Next session must start at **phase 6** (Matchmaker-triggered mock
    provision) and stop there.
- Verified git state at end: this commit on local `main`; working tree should
  be clean; still ahead of `origin/main`; did not push.
