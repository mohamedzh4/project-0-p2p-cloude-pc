# Handoff — phase 29 Linux Pro VFIO bind apply (2026-09-07)

- Phase goal: When Linux Pro is offered and Matchmaker POSTs provision, apply
  vfio-pci against injectable/live sysfs with rollback. Keep the window
  plan-only. Do not attach GPU to QEMU, invent locators, or apply Hyper-V.
- Verified git state at start: `d6a8a94` on local `main` (phase 28). Did not
  rewrite prior commits. Did not push. Did not touch `pr/*`. Did not invent
  hardware logs. `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.

## Window vs provision

- **Window / eligibility** (`commands.rs::plan_gpu_bind`,
  `evaluate_hosting_offers`): `WINDOW_GPU_BIND_APPLY` = `apply: false`.
  Opening settings must not steal the GPU.
- **Provision** (`POST /v1/provision`): `apply: true` only when
  `linux_pro_provision_should_apply` (Linux + Pro + probe offered Pro) and
  `ProvisionGate.vfio` is set. Production sets that gate when Linux Pro is
  offered (`PciSysfs::live()` + `SysfsBindOps`). Tests inject a mock tree.
  Ungated Pro still returns the Sunshine stub without writing sysfs (same
  pattern as Standard without Firecracker/QEMU boot inputs).
- Windows Pro stays plan-only on this path (Hyper-V apply-from-provision is
  next). Standard Linux/Windows ignore a VFIO gate.

## Sysfs sequence + rollback

Kernel vfio.rst + PCI `new_id` (`vendor device` hex). Same plan as phase 22:

1. `driver_override` ← `vfio-pci`
2. Host driver `unbind` ← BDF (`DDDD:BB:DD.F`)
3. vfio-pci `new_id` ← `vendor device`
4. vfio-pci `bind` ← BDF

Writes are kernel sysfs commands (write, not append/create). Mid-step failure
unbinds vfio-pci and rebinds the previous driver. `exclusive: true` stays.
`hypervisorInvoked` stays false. Locator stays
`https://provision.invalid/qemu-vfio`. `cfg(test)` still refuses live `/sys`.

## Files created/changed

- `host-agent/src-tauri/src/gpu_bind.rs`, `provision_http.rs`, `commands.rs`,
  `lib.rs`, `provision.rs`
- `host-agent/src/types.ts`, `CapabilityPanel.tsx`
- `host-agent/README.md`, `host-agent/.env.example`, `guest-images/README.md`
- `docs/runbooks/gate-pro.md`
- `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`, architecture
  snippet
- `docs/phase-logs/phase-29-linux-vfio-apply.md` (this file)

## Test results

- Host Agent: **296 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
  Was 288 (+8). No live `/sys`, no QEMU VFIO attach.
- Matchmaker: not run (untouched). Still 162.
- Web: not run (untouched). Vitest still 31.
- Guest-images: not run (untouched). Unittest still 12.

New tests:

- `gpu_bind::tests::window_apply_is_false_linux_pro_provision_apply_is_true_when_offered`
- `gpu_bind::tests::given_linux_pro_probe_apply_true_issues_unbind_new_id_and_bind`
- `provision_http::tests::linux_pro_with_vfio_gate_applies_bind_and_keeps_the_placeholder_locator`
- `provision_http::tests::linux_pro_vfio_mid_step_failure_is_400_rolls_back_and_does_not_occupy`
- `provision_http::tests::linux_pro_vfio_gate_with_live_sysfs_is_refused_in_tests`
- `provision_http::tests::linux_pro_gated_without_vfio_inputs_stays_stub`
- `provision_http::tests::linux_standard_ignores_vfio_gate`
- `provision_http::tests::windows_pro_ignores_vfio_gate`

## Flagged for the next agent

- **Next: Hyper-V DDA/GPU-P apply from the provision path** (not plan-only
  window), then locators, then web. Not a fake live Kasm URL.
- Hardware lines stay **Implemented in code. Not yet run on physical hardware.**
- Locator still placeholder. No QEMU `-device vfio-pci` this phase.
- Successful bind leaves the GPU on vfio-pci with no teardown port.
- Aikido: try MCP; if error, not a scan pass. Run `/aikido:setup`.
- `origin/main` still behind. Nothing pushed.
