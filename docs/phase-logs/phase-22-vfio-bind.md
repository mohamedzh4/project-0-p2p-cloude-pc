# Handoff — phase 22 VFIO/DDA bind with rollback (2026-09-06)

- Phase goal: Plan (and, on a mock sysfs tree, apply) exclusive Pro GPU
  assignment with rollback. Linux is vfio-pci; Windows is DDA/GPU-P log-only.
  Do not boot. Do not invent metal output.
- Verified git state at start: `cc6e2ae` on local `main`, working tree clean,
  ahead of `origin/main`. Did not invent hardware logs. Did not create
  `gate-*-result.md`. `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.
  Did not push. Did not start phase 23.
- Files created/changed:
  - `host-agent/src-tauri/src/gpu_bind.rs` (new; no Tauri; injectable sysfs)
  - `host-agent/src-tauri/src/lib.rs`, `commands.rs`
  - `host-agent/src/types.ts`, `api.ts`, `App.tsx`
  - `host-agent/src/components/CapabilityPanel.tsx`
  - `host-agent/README.md`
  - `PHASE_PLAN.md` (phase 22 done; 22b leftover)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-22-vfio-bind.md` (this file)
- Key decisions made and why:
  - **Injectable sysfs/driver paths.** `PciSysfs` + `write_driver_unbind` /
    `write_driver_bind` / `write_vfio_new_id` take explicit files. Tests build
    a temp tree (UUID → BDF, iommu_group, nvidia/vfio-pci control files). CI
    never needs root or a real GPU. Tests refuse apply against live `/sys`.
  - **Plan in the window; apply only in tests.** Stub provision still does
    not bind a volunteer GPU (that would unbind the host driver without a
    guest). `plan_gpu_bind` is apply=false. Happy path does not ask for
    `lspci` — `resolve_pci_address` walks sysfs (`dcl_gpu_id` / `gpu_uuid` /
    NVIDIA proc UUID / BDF).
  - **Rollback on bind failure.** Unbind host driver → `new_id` → bind
    vfio-pci. If vfio bind fails, unbind vfio-pci and rebind the previous
    driver. Ineligible (no IOMMU, no second display path, shareGpu off,
    missing iommu_group) returns before any write.
  - **Windows is 22b.** DDA/GPU-P is `WouldAssign` log only. No PowerShell
    (must not hang). Ineligible Windows is plain language (second display
    path). Live Hyper-V apply is leftover **22b**.
  - **Never Firecracker for Pro.** Linux Pro stays QEMU+VFIO; macOS skips
    instead of aliasing to Firecracker. GPU exclusivity copy is unchanged.
    `HostSelector.select`, occupancy indexes, CSP, Argon2id, JWT untouched.
- Test results:
  - Host Agent: **208 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 193. Added mock-sysfs plan, bind success, bind fail → rollback,
    IOMMU / second-display / shareGpu / missing group refuse with no partial
    bind, Firecracker never selected, Windows would-assign, Windows
    ineligible, macOS skip, live-sysfs forbidden in tests, UUID resolve,
    injectable driver-path writes.
  - Matchmaker: **161 passed**, 0 failed. Unchanged.
  - Web: **31 passed**, 0 failed. Host Agent `npm run typecheck` passed.
- What’s built / verified / needs hardware / still paper:
  - **Built:** Linux VFIO plan/apply-on-mock with rollback; Windows DDA/GPU-P
    plan/log; Host Agent “Pro GPU assignment” card.
  - **Verified:** test counts above. Preview on `localhost:1420` shows
    exclusive GPU copy and a VFIO plan when GPU share is on (sample Linux
    data); Mac live probe still does not offer Pro.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO remain **OPEN**.
  - **Still paper:** live VFIO on metal; Windows DDA apply (**22b**);
    insufficient-resource UX (phase 23); Windows QEMU silent install
    (**19b**); pairwise ACLs; live guest Tailscale/rclone; FR13 `latency_ms`.
- Flagged for the next agent:
  - Start at **phase 23** (insufficient-resource rejection UX). Do not invoke
    Firecracker/QEMU. Do not mark gates closed. Do not create
    `gate-*-result.md`. Do not boot a VM. Do not use Firecracker for Pro.
    Do not spawn PowerShell for DDA (that is **22b**).
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; still ahead of
  `origin/main`; did not push.
