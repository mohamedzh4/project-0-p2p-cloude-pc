# Handoff — phase 27 Firecracker invoke (2026-09-07)

- Phase goal: Linux Standard `POST /v1/provision` actually starts the
  already-fetched Firecracker binary and `InstanceStart`s a microVM from the
  phase-21 launch config + verified Standard `rootfs.img`. No QEMU/WHPX boot,
  no live VFIO from the window, no Hyper-V DDA apply-from-window, no fake
  live Kasm URL, no web loop.
- Verified git state at start: `1f5890e` on local `main` (phase 26). Did not
  rewrite `1f5890e` / `8ededf8` / `a2be066`. Did not push. Did not touch
  `pr/*`.

## What “real invoke” means here

- Binary: phase-19 cache path (`hypervisor.rs::ensure` still never execs).
- Argv (locked): `{binary} --api-sock <sock> --enable-pci`
  (Firecracker v1.16.1 getting-started; jailer is **not** used).
- Socket: `{image_cache}/firecracker/api.sock`.
- API order (locked, swagger 204):
  1. `PUT /boot-source` `{ kernel_image_path, boot_args }`
     (`console=ttyS0 reboot=k panic=1`; aarch64 prefixes `keep_bootcon`)
  2. `PUT /drives/rootfs` `{ drive_id: "rootfs", path_on_host, is_root_device: true, is_read_only: false }`
  3. `PUT /machine-config` `{ vcpu_count, mem_size_mib, smt: false }`
     (clamped sliders; Firecracker defaults are 1 vCPU / 128 MiB)
  4. `PUT /actions` `{ action_type: "InstanceStart" }`
- No TAP, vsock, or logger PUT. Child is killed on API failure; detached on
  success (no teardown HTTP yet).
- Kernel: `FIRECRACKER_KERNEL` (fail if set but missing) → launch
  `kernelPath` → cache `{tier}/vmlinux` → sibling of rootfs named `vmlinux`.
  Missing kernel/binary/rootfs fails **before** spawn. Packer does not invent
  vmlinux from GRUB.
- Locator stays `https://provision.invalid/firecracker`.
  `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`. Phase 28+ (or a
  later locator phase) is when that seed dies — do not replace it with a
  fake live URL.
- `plan()` / `plan_provision` stay stubs (`hypervisorInvoked: false`).
  Invoke is HTTP-port-only when Linux Standard is offered and
  `ProvisionGate.firecracker` is set. macOS/Windows/Pro never spawn.

## Stub / mock / log-only inventory (updated)

- **Firecracker invoke — this phase (software contract).** Real InstanceStart
  on metal still needs KVM, a real `rootfs.img`, and operator `vmlinux`.
  Tests inject `RecordingFirecracker` (fake argv + HTTP sequence). Production
  `ProcessSpawner` is refused under `cfg(test)` and off Linux.
- **QEMU WHPX boot — stub.** Still no `qemu-system-*.exe -accel whpx`.
- **VFIO / Hyper-V DDA from the window — plan-only.** Unchanged.
- **Live Kasm locator — placeholder.** No TAP/net in this phase, so the guest
  cannot expose Kasm even if InstanceStart returns 204 on metal.
- **Jailer — leftover.** Not used.

## Files created/changed

- `host-agent/src-tauri/src/firecracker.rs` (new)
- `host-agent/src-tauri/src/lib.rs`, `provision_http.rs`, `provision.rs`,
  `launch.rs`, `commands.rs`
- `matchmaker/src/matchmaker/provision.py` + `tests/test_provision.py`
  (accept `hypervisorInvoked: true` only when the locator hostname ends with
  `.invalid`; refuse invoke + a live URL)
- `host-agent/.env.example`, `host-agent/README.md`, `guest-images/README.md`
- `docs/runbooks/gate-fc.md` (operator requirements, not “confirmed working”)
- `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`, architecture
  snippet
- `docs/phase-logs/phase-27-firecracker-invoke.md` (this file)

## Key decisions

- Jailer out of scope; `--enable-pci` matches v1.16.1 getting-started.
- Do not put a fake binary at `{cache}/firecracker` (that collides with
  `api.sock` under `{cache}/firecracker/`).
- Matchmaker must not treat InstanceStart as a live desktop. Placeholder
  locators may travel with `hypervisorInvoked: true`; a non-`.invalid` URL
  with invoke is dropped (seeded locator kept).
- If Linux Standard boot inputs are attached and kernel/rootfs/binary are
  missing, HTTP is **400**, not a successful stub.

## Test results

- Host Agent: **266 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
  Was 246 (+20). No real Firecracker, no `/dev/kvm`.
- Matchmaker: **162 passed**, 0 failed (`matchmaker/.venv/bin/pytest`).
  Was 161 (+1 net: two new tests replace the old refuse-any-invoke test).
- Web: not run (untouched). Vitest still 31.
- Guest-images: not run (untouched). Unittest still 12.

Argv/API lock tests (no `/dev/kvm`):

- `firecracker::tests::linux_standard_invoke_locks_argv_and_api_order`
- `firecracker::tests::missing_kernel_fails_before_spawn`
- `firecracker::tests::macos_does_not_spawn_firecracker`
- `firecracker::tests::windows_does_not_spawn_firecracker`
- `firecracker::tests::probe_without_kvm_does_not_spawn`
- `firecracker::tests::unix_http_put_talks_http_over_a_fake_socket`
- `firecracker::tests::production_spawner_is_refused_under_cfg_test`
- `provision_http::tests::linux_standard_with_boot_inputs_invokes_and_keeps_the_placeholder_locator`
- `provision_http::tests::linux_standard_boot_without_kernel_is_400_not_a_successful_stub`
- `provision_http::tests::windows_standard_ignores_firecracker_boot_inputs`

Matchmaker:

- `test_http_provisioner_accepts_invoke_when_locator_is_still_a_placeholder`
- `test_http_provisioner_refuses_a_live_locator_claimed_with_invoke`

## Operator requirements (metal — not confirmed working)

These are unattended blockers. Do not write “confirmed working” after reading
this list.

- Linux host with KVM loaded and `/dev/kvm` readable/writable (capability
  probe must offer Standard).
- Phase-19 Firecracker v1.16.1 binary already fetched into the user cache.
- Verified Standard `rootfs.img` (phase 20 fetch / phase 26 Packer output).
- Operator `vmlinux` via `FIRECRACKER_KERNEL` (or next to the rootfs). The
  Firecracker demo kernel is not enough for Docker/Kasm.
- TAP/network is **not** this phase → even a successful InstanceStart does
  not yield a reachable Kasm URL.

## Flagged for the next agent

- **Next phase: Windows QEMU `-accel whpx` boot** (remaining stub). Not live
  VFIO from the window, not Hyper-V DDA apply-from-window, not a fake live
  Kasm URL, not the web loop.
- Hardware lines in `PROJECT_STATUS.md` stay **Implemented in code. Not yet
  run on physical hardware.**
- Locator still placeholder. `SEED_HOST_KASM_URL` still `example.invalid`.
- Aikido MCP `namespaceStatus: error`; not a scan pass. Run `/aikido:setup`.
- `origin/main` still behind. Nothing pushed.
