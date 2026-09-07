# Handoff — phase 30 Windows Pro Hyper-V DDA/GPU-P apply from provision (2026-09-07)

- Phase goal: When Windows Pro is offered and Matchmaker POSTs provision, apply
  exclusive DDA (preferred) or exclusive GPU-P through the production PowerShell
  runner, with rollback. Keep the window plan-only. Do not `Start-VM`, invent
  locators, or rewrite VFIO / Firecracker / QEMU WHPX.
- Verified git state at start: `7b9d12e` on local `main` (phase 29). Did not
  rewrite prior commits. Did not push. Did not touch `pr/*`. Did not invent
  hardware logs. `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.

## Window vs provision

- **Window / eligibility** (`commands.rs::plan_gpu_bind`,
  `evaluate_hosting_offers`): `WINDOW_GPU_BIND_APPLY` = `apply: false`,
  `hyperv: None`. Opening settings must not spawn PowerShell or steal the GPU.
- **Provision** (`POST /v1/provision`): `apply: true` only when
  `windows_pro_provision_should_apply` (Windows + Pro + probe offered Pro) and
  `ProvisionGate.hyperv` is set. Production sets that gate when Windows Pro is
  offered (`ProcessHyperVGpuOps`). Tests inject `RecordingHyperV`. Ungated Pro
  still returns the Sunshine stub without invoking cmdlets (same pattern as
  Linux Pro without VFIO inputs).
- Linux Pro VFIO gate is unchanged. Standard Linux/Windows ignore a Hyper-V
  gate.

## DDA vs GPU-P + rollback

Documented Microsoft cmdlets (phase 25; Context7 Hyper-V module). No new
cmdlets. No `Start-VM`. VM name `dcl-pro-guest` must already exist and be
powered off.

DDA preferred when a location path exists:

1. `Set-VM` (AutomaticStopAction TurnOff, GuestControlledCacheTypes,
   Low/HighMemoryMappedIoSpace) — not rolled back
2. `Disable-PnpDevice -Confirm:$false`
3. `Dismount-VMHostAssignableDevice -Force -LocationPath`
4. `Add-VMAssignableDevice -LocationPath -VMName`
5. `Get-VMAssignableDevice` must confirm

DDA rollback: `Remove-VMAssignableDevice` → `Mount-VMHostAssignableDevice` →
`Enable-PnpDevice -Confirm:$false`.

GPU-P only when DDA has no location path and partition count 1 is valid
(`exclusive: true`):

1. `Set-VMHostPartitionableGpu -PartitionCount 1`
2. `Add-VMGpuPartitionAdapter -VMName -InstancePath`
3. `Set-VMGpuPartitionAdapter` min/max/optimal 100000000
4. `Get-VMGpuPartitionAdapter` must confirm

GPU-P rollback: `Remove-VMGpuPartitionAdapter` → restore partition count.

`hypervisorInvoked` stays false. Locator stays
`https://provision.invalid/hyperv-gpu`. Sunshine stays
`https://provision.invalid/sunshine`. `cfg(test)` refuses live PowerShell.
Windows Pro image fetch stays would-fetch (assignment attaches to an existing
VM, not a fetched disk).

## Files created/changed

- `host-agent/src-tauri/src/gpu_bind.rs`, `provision_http.rs`, `commands.rs`,
  `lib.rs`, `provision.rs`
- `host-agent/src/types.ts`, `CapabilityPanel.tsx`
- `host-agent/README.md`, `host-agent/.env.example`, `guest-images/README.md`
- `docs/runbooks/gate-pro.md`
- `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`, architecture
  snippet
- `docs/phase-logs/phase-30-hyperv-provision-apply.md` (this file)

## Test results

- Host Agent: **305 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
  Was 296 (+9). No live PowerShell, no `Start-VM`.
- Matchmaker: not run (untouched). Still 162.
- Web: not run (untouched). Vitest still 31.
- Guest-images: not run (untouched). Unittest still 12.

New tests:

- `gpu_bind::tests::window_apply_is_false_windows_pro_provision_apply_is_true_when_offered`
- `gpu_bind::tests::given_windows_pro_probe_apply_true_issues_dda_cmdlets`
- `provision_http::tests::windows_pro_with_hyperv_gate_applies_dda_and_keeps_the_placeholder_locator`
- `provision_http::tests::windows_pro_gpu_p_gate_applies_exclusive_partition`
- `provision_http::tests::windows_pro_hyperv_mid_step_failure_is_400_rolls_back_and_does_not_occupy`
- `provision_http::tests::windows_pro_hyperv_gate_with_live_ops_is_refused_in_tests`
- `provision_http::tests::windows_pro_gated_without_hyperv_inputs_stays_stub`
- `provision_http::tests::windows_standard_ignores_hyperv_gate`
- `provision_http::tests::linux_pro_ignores_hyperv_gate`

## Flagged for the next agent

- **Next: live locators** — once a VM would have Kasm/Sunshine reachable, Host
  Agent returns the real URL to Matchmaker and drop `SEED_HOST_KASM_URL` from
  the provision flow (still not “confirmed on hardware”). Then web.
- Hardware lines stay **Implemented in code. Not yet run on physical hardware.**
- Locator still placeholder. No `Start-VM`. No live Sunshine URL.
- **Minimal unavoidable manual steps on a real Windows host:** (1) enable the
  Hyper-V Windows feature if missing (Windows may reboot); (2) approve UAC so
  the Host Agent runs elevated (`Disable-PnpDevice` /
  `Dismount-VMHostAssignableDevice` need admin; this agent does not
  auto-elevate); (3) have a Hyper-V VM named `dcl-pro-guest` already created
  and powered off.
- Successful assign leaves the GPU on the VM with no unassign teardown port.
- Aikido: try MCP; if error, not a scan pass. Run `/aikido:setup`.
- `origin/main` still behind. Nothing pushed.
