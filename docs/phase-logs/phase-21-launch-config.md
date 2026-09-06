# Handoff — phase 21 launch config from sliders + GPU id (2026-09-06)

- Phase goal: Map clamped v2 sharing sliders (and a dedicated GPU id) to a
  launch-config JSON for the stub backend already selected in `provision.rs`.
  Do not boot.
- Verified git state at start: `df02689` on local `main`, working tree clean,
  ahead of `origin/main`. Did not invent hardware logs. Did not create
  `gate-*-result.md`. `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.
  Did not push. Did not start phase 22.
- Files created/changed:
  - `host-agent/src-tauri/src/launch.rs` (new; no Tauri; never invokes)
  - `host-agent/src-tauri/src/image.rs` (public cache paths for rootfs/kernel)
  - `host-agent/src-tauri/src/lib.rs`, `commands.rs`
  - `host-agent/src/types.ts`, `api.ts`, `App.tsx`
  - `host-agent/src/components/CapabilityPanel.tsx`
  - `host-agent/README.md`
  - `PHASE_PLAN.md` (phase 21 done)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-21-launch-config.md` (this file)
- Key decisions made and why:
  - **Reuse `provision.rs` backends.** Linux Standard → Firecracker, Windows
    Standard → QEMU+WHPX, Linux Pro → QEMU+VFIO, Windows Pro → Hyper-V GPU.
    macOS / other OS still fail as unsupported (not a Firecracker fallback).
  - **Sliders are the source of truth after clamp.** `vcpuCount` and
    `memoryMib` come from `SharingConfig::clamped_to`. Values above this PC
    are clamped down before emit. Below the documented Kasm minimum
    (2 vCPU / 4 GiB in `docs/runbooks/gate-fc.md`) is a plain-language `Err`.
    Phase 23 owns the full rejection UX.
  - **GPU is exclusive or omitted.** Standard omits `gpu` even if
    `shareGpu` / `sharedGpuId` are stored. Pro includes
    `{ id, exclusive: true }` only when `shareGpu` is on. Never a concurrent
    slice.
  - **Phase-20 paths when present.** Rootfs from guest-image status (matching
    lab tier) or `{cache}/{tier}/rootfs.img`. Kernel from `{cache}/{tier}/vmlinux`
    or a sibling of the rootfs. Missing files leave the fields null; they do
    not fail the config.
  - **Never invoked.** `hypervisorInvoked` is always false. Tests assert JSON
    fields, not a running VM. The webview does not fetch. `HostSelector.select`,
    occupancy indexes, GPU exclusivity copy, CSP, Argon2id, JWT unchanged.
- Test results:
  - Host Agent: **193 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 176. Added slider JSON, peer backends, Standard omits GPU, Pro exclusive
    GPU, shareGpu required for Pro, macOS unsupported, min vCPU/RAM reject,
    clamp-before-emit, phase-20 cache kernel/rootfs, guest-status rootfs.
  - Matchmaker: **161 passed**, 0 failed. Unchanged.
  - Web: **31 passed**, 0 failed. Host Agent `npm run typecheck` passed.
- What’s built / verified / needs hardware / still paper:
  - **Built:** launch-config struct/JSON from clamped sliders; UI cards.
  - **Verified:** test counts above. Preview on `localhost:1420` shows Standard
    vCPU/RAM from sliders and Pro refused until GPU share is on.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO remain **OPEN**.
  - **Still paper:** VFIO/DDA bind (phase 22); insufficient-resource UX
    (phase 23); Windows QEMU silent install (**19b**); pairwise ACLs; live
    guest Tailscale/rclone; FR13 `latency_ms`.
- Flagged for the next agent:
  - Start at **phase 22** (VFIO/DDA bind with rollback). Do not invoke
    Firecracker/QEMU. Do not mark gates closed. Do not create
    `gate-*-result.md`. Do not boot a VM. Do not use Firecracker for Pro.
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; still ahead of
  `origin/main`; did not push.
