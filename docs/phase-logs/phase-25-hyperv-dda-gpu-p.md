# Handoff — phase 25 Hyper-V DDA/GPU-P apply with rollback (2026-09-06)

- Phase goal: Execute exclusive Hyper-V DDA (preferred) or GPU-P assignment
  through an injectable PowerShell/CIM runner, with VFIO-style rollback, without
  booting a guest and without live Hyper-V on this Mac.
- Verified git state at start: `a2be066` on local `main` (phase 24 / leftover
  19b). Did not rewrite that commit. Did not invent hardware logs. Did not
  create `gate-*-result.md`. `SEED_HOST_KASM_URL` stays
  `https://example.invalid/kasm`. Did not push. Did not rebase `pr/*`. Did
  not close gates.
- Files created/changed:
  - `host-agent/src-tauri/src/gpu_bind.rs` (Windows apply + injectable
    `HyperVGpuOps`; documented Microsoft cmdlets)
  - `host-agent/src-tauri/src/commands.rs` (window stays `apply: false`)
  - `host-agent/src/types.ts`, `host-agent/src/components/CapabilityPanel.tsx`
  - `host-agent/README.md`, `host-agent/.env.example`
  - `docs/runbooks/gate-pro.md` (cmdlets + DDA plan copy), `gate-fc.md` /
    `gate-whpx.md` (automation pointer 18–25)
  - `PHASE_PLAN.md` (phase 25 done; **22b** closed)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-25-hyperv-dda-gpu-p.md` (this file)
- Key decisions made and why:
  - **Official cmdlets, not guessed.** DDA follows Microsoft “Deploy graphics
    devices using Discrete Device Assignment”: location path from
    `Get-PnpDeviceProperty DEVPKEY_Device_LocationPaths`, then
    `Disable-PnpDevice -Confirm:$false`, `Dismount-VMHostAssignableDevice
    -Force -LocationPath`, `Add-VMAssignableDevice -LocationPath -VMName`,
    verify with `Get-VMAssignableDevice`. Rollback is
    `Remove-VMAssignableDevice`, `Mount-VMHostAssignableDevice`,
    `Enable-PnpDevice -Confirm:$false`. VM prep is `Set-VM -AutomaticStopAction
    TurnOff`, `GuestControlledCacheTypes`, `LowMemoryMappedIoSpace 3Gb`,
    `HighMemoryMappedIoSpace 33280Mb` (not rolled back; not a host GPU bind).
  - **GPU-P only when exclusive.** If there is no DDA location path,
    `Get-VMHostPartitionableGpu` must list partition count `1`. Then
    `Set-VMHostPartitionableGpu -PartitionCount 1`,
    `Add-VMGpuPartitionAdapter -VMName -InstancePath`,
    `Set-VMGpuPartitionAdapter` min/max/optimal 100000000 for VRAM/Encode/
    Decode/Compute. A shared slice the host still renders with is refused.
    Rollback is `Remove-VMGpuPartitionAdapter` and restore partition count.
  - **Injectable runner, hang-safe production path.** Tests record cmdlets and
    never spawn PowerShell (`LiveHyperVForbiddenInTests` if
    `ProcessHyperVGpuOps` is used under `cfg(test)`). Production is
    `powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass
    -Command` with a 30s kill. The window stays plan-only so opening the
    agent does not steal the volunteer GPU.
  - **Never claim assigned without verify.** Empty `Get-VMAssignableDevice` /
    `Get-VMGpuPartitionAdapter` rolls back and returns `ApplyFailed`.
  - **Do not execute a guest.** No `Start-VM`. Default VM name `dcl-pro-guest`
    must already exist on metal; this phase does not create it. Firecracker is
    never used for Pro. `HostSelector.select`, occupancy indexes, CSP,
    Argon2id, JWT unchanged.
- Test results:
  - Host Agent: **246 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 238. Added Windows DDA plan (no invoke), DDA success sequence, DDA
    mid-step fail → Mount/Enable rollback with no leftover assigned state,
    GPU-P exclusive success, GPU-P mid-step fail → remove/restore rollback,
    apply without runner not assigned, live PowerShell forbidden in tests,
    documented cmdlet flags, hang-safe timeout copy.
  - Matchmaker: **161 passed**, 0 failed. Unchanged (not run this session;
    no Matchmaker files touched).
  - Web: **31 passed**, 0 failed. Unchanged (not run this session; no web
    files touched). Host Agent `npm run typecheck` after UI copy change.
- What’s built / verified / needs hardware / still paper:
  - **Built:** Windows DDA/GPU-P apply + rollback on an injected runner;
    production PowerShell argv + timeout (not executed on this Mac).
  - **Verified:** test counts above. This Mac cannot run Hyper-V.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO remain **OPEN**.
  - **Still paper:** live VFIO on metal; live Hyper-V assignment on metal;
    live QEMU+WHPX boot; pairwise ACLs; live guest Tailscale/rclone; FR13
    `latency_ms`. Guest image on Windows is still would-fetch (phase 20).
- Flagged for the next agent:
  - Hardware gates on Linux/Windows metal. Do not invoke Firecracker/QEMU.
    Do not mark gates closed. Do not create `gate-*-result.md`. Do not boot
    a VM. Do not use Firecracker for Pro. Do not bind live `/sys` from tests.
    Do not spawn live PowerShell from tests.
  - **Minimal unavoidable manual steps on a real Windows host:** (1) enable
    the Hyper-V Windows feature if missing (Windows may reboot); (2) approve
    UAC so the Host Agent runs elevated (`Disable-PnpDevice` /
    `Dismount-VMHostAssignableDevice` need admin; this phase does not
    auto-elevate); (3) have a Hyper-V VM named `dcl-pro-guest` already
    created and powered off — this agent does not create or `Start-VM` it.
    QEMU NSIS UAC from phase 24 still applies for Standard.
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed. Do not rebase existing
    `pr/*` branches.
- Verified git state at end: this commit on local `main`; phase 24 `a2be066`
  still in history; still ahead of `origin/main`; did not push.
