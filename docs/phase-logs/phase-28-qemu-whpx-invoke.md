# Handoff — phase 28 QEMU WHPX invoke (2026-09-07)

- Phase goal: Windows Standard `POST /v1/provision` actually starts the
  already-fetched Weilnetz QEMU binary with documented `-accel whpx` and the
  verified Standard disk. No live VFIO from the window, no Hyper-V DDA
  apply-from-window, no Hyper-V Standard boot, no fake live Kasm URL, no web
  loop.
- Verified git state at start: `0cce3e0` on local `main` (phase 27). Did not
  rewrite `0cce3e0` / `1f5890e`. Did not push. Did not touch `pr/*`.

## What “real invoke” means here

- Binary: phase-24 cache path (`hypervisor.rs::ensure` still never execs).
- Argv (locked, qemu.org `docs/system/whpx` quick start):

  x86_64:
  `{qemu-system-x86_64.exe} -accel whpx -M pc -smp cores={vcpu}
  -m {size} -device ich9-usb-ehci1 -device usb-tablet -hda {disk}`

  aarch64:
  `{qemu-system-aarch64.exe} -accel whpx -M virt -cpu host -smp cores={vcpu}
  -m {size} -bios edk2-aarch64-code.fd -device ramfb -device nec-usb-xhci
  -device usb-kbd -device usb-tablet -hda {disk}`

  `{size}` is clamped slider MiB as `NG` when whole GiB, else `NM`.
  `{vcpu}` is clamped slider mapped to `-smp cores=N`. `{disk}` is the
  phase-20/26 verified cache file (`rootfs.img`; QEMU `-hda` auto-detects
  raw or qcow2 content).
- No TCG. Probe WHPX ≠ Available → structured error before spawn.
- No Hyper-V `Start-VM` (Hyper-V remains the Pro GPU backend).
- Child is detached on spawn success (no teardown HTTP yet).
- Locator stays `https://provision.invalid/qemu-whpx`.
  `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.
- `plan()` / `plan_provision` stay stubs (`hypervisorInvoked: false`).
  Invoke is HTTP-port-only when Windows Standard is offered and
  `ProvisionGate.qemu` is set. macOS/Linux/Pro never spawn QEMU.

## Windows image fetch (this phase, required for boot)

- `image.rs::ensure_windows` now locate/fetch/verifies the Standard QEMU
  disk with the same SHA-256 + Ed25519 contract as Linux phase 20.
- Windows Pro image is still would-fetch (Hyper-V GPU, not this phase).
- Cache name stays `{cache}/standard/rootfs.img`.

## Stub / mock / log-only inventory (updated)

- **QEMU WHPX invoke — this phase (software contract).** Real boot on metal
  still needs WHPX (HypervisorPlatform), Weilnetz QEMU, UAC for the NSIS
  install, and a real verified disk. Tests inject `RecordingQemu`. Production
  `ProcessSpawner` is refused under `cfg(test)` and off Windows.
- **Firecracker invoke — phase 27.** Unchanged.
- **VFIO / Hyper-V DDA from the window — plan-only.** Unchanged.
- **Live Kasm locator — placeholder.** No TAP/net; spawn does not yield a
  reachable Kasm URL.
- **Jailer — leftover.** Not used.

## Files created/changed

- `host-agent/src-tauri/src/qemu.rs` (new)
- `host-agent/src-tauri/src/lib.rs`, `provision_http.rs`, `provision.rs`,
  `image.rs`, `hypervisor.rs`
- `host-agent/.env.example`, `host-agent/README.md`, `guest-images/README.md`
- `docs/runbooks/gate-whpx.md`
- `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`, architecture
  snippet
- `docs/phase-logs/phase-28-qemu-whpx-invoke.md` (this file)

## Key decisions

- Documented qemu.org WHPX flags only; do not invent TCG or Hyper-V Standard
  boot.
- Windows Standard image fetch became real because boot cannot work without a
  verified disk. Do not expand into Pro GPU fetch/apply.
- Matchmaker already accepts `hypervisorInvoked: true` with a `.invalid`
  hostname (phase 27). No Matchmaker code change.

## Test results

- Host Agent: **288 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
  Was 266 (+22). No real QEMU, no WHPX.
- Matchmaker: not run (untouched). Still 162.
- Web: not run (untouched). Vitest still 31.
- Guest-images: not run (untouched). Unittest still 12.

Argv lock tests (no WHPX):

- `qemu::tests::windows_standard_invoke_locks_documented_whpx_argv`
- `qemu::tests::aarch64_locks_documented_whpx_argv`
- `qemu::tests::probe_without_whpx_does_not_spawn_and_does_not_use_tcg`
- `qemu::tests::linux_does_not_spawn_qemu`
- `qemu::tests::macos_does_not_spawn_qemu`
- `provision_http::tests::windows_standard_with_boot_inputs_invokes_whpx_and_keeps_the_placeholder_locator`
- `provision_http::tests::windows_standard_boot_without_disk_is_400_not_a_successful_stub`
- `image::tests::windows_fetches_standard_qemu_image_when_missing`

## Operator requirements (metal — not confirmed working)

These are unattended blockers. Do not write “confirmed working” after reading
this list.

- Windows host with Windows Hypervisor Platform enabled so the capability
  probe offers Standard (`DISM /online /Enable-Feature /FeatureName:HypervisorPlatform /All`).
- Phase-24 Weilnetz QEMU already installed into the user cache (`/S` `/D=`).
  UAC consent is still manual.
- Verified Standard disk (phase 20 fetch / phase 26 Packer `rootfs.img` or
  qcow2 content stored as `rootfs.img`).
- TAP/network is **not** this phase → even a successful spawn does not yield
  a reachable Kasm URL.

## Flagged for the next agent

- **Next: live VFIO bind apply (Linux Pro)** on real sysfs with rollback.
  Then locators, then web. Not a fake live Kasm URL.
- Hardware lines in `PROJECT_STATUS.md` stay **Implemented in code. Not yet
  run on physical hardware.**
- Locator still placeholder. `SEED_HOST_KASM_URL` still `example.invalid`.
- Aikido: MCP `namespaceStatus: error`; not a scan pass. Run `/aikido:setup`.
- `origin/main` still behind. Nothing pushed.
