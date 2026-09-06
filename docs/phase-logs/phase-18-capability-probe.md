# Handoff — phase 18 host capability probe (2026-09-06)

- Phase goal: Probe whether this PC can host Standard/Pro (OS, KVM, WHPX,
  IOMMU, GPU / second display) in Host Agent Rust, and correct hardware-gate
  wording to OPEN pending real-hardware verification.
- Verified git state at start: local `main` after phase 17 software (working
  tree already had uncommitted phase-17 files). Did not invent hardware logs.
  Did not create `gate-*-result.md`. `SEED_HOST_KASM_URL` stays
  `https://example.invalid/kasm`. Did not push.
- Files created/changed:
  - `host-agent/src-tauri/src/host_capabilities.rs` (new; no Tauri types)
  - `host-agent/src-tauri/src/lib.rs`, `commands.rs`
  - `host-agent/src/types.ts`, `api.ts`, `preview.ts`, `App.tsx`, `styles.css`
  - `host-agent/src/components/CapabilityPanel.tsx`
  - `host-agent/README.md`
  - `docs/runbooks/gate-fc.md`, `gate-whpx.md`, `gate-pro.md`
  - `docs/runbooks/headscale.md`, `docs/runbooks/renterd.md`
  - `docs/phase-logs/phase-gates-operator-pass.md` (retraction note only)
  - `docs/phase-logs/phase-17-complete-software.md` (later note only)
  - `PHASE_PLAN.md` (phases 18–23; gates OPEN)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`, `web/README.md`
  - `docs/phase-logs/phase-18-capability-probe.md` (this file)
- Key decisions made and why:
  - **Gates OPEN.** 2026-09-05 operator PASS was retracted as unverified.
    Current wording is **OPEN — pending real-hardware verification**. Historical
    `phase-gates-operator-pass.md` kept; added a retraction note.
  - **Probe, do not ask.** `evaluate` takes injected OS/KVM/WHPX/IOMMU/GPU so
    this Mac and Linux-without-KVM CI do not need metal. Live Linux reads
    `/dev/kvm` and IOMMU sysfs. Live Windows WHPX is `unknown` (no QEMU).
    UI copy never names Firecracker/QEMU/WHPX or asks which backend.
  - **Second display path** = iGPU name heuristic or 2+ GPUs. Pro on Linux also
    needs IOMMU present. Windows IOMMU stays optional/unknown.
  - **GPU exclusivity copy unchanged.** `HostSelector.select`, occupancy
    indexes, CSP, Argon2id, JWT untouched. No hypervisor invoked.
- Test results:
  - Host Agent: **143 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 126. Added Mac / Linux-without-KVM not offered, two GPUs → second
    display path, KVM path mock, IOMMU sysfs mock, Windows WHPX stub.
  - Matchmaker: **161 passed**, 0 failed. Unchanged.
  - Web: **31 passed**, 0 failed. Host Agent `npm run typecheck` passed.
  - Host Agent preview (`localhost:1420`): capability panel shows Standard/Pro
    available on sample dual-GPU Linux data; exclusivity copy still present;
    pause / in-session / GPU picker still work. Favicon 404 only.
- What’s built / verified / needs hardware / still paper:
  - **Built:** capability probe + Host Agent “What this PC can host” panel;
    phases 19–23 listed in `PHASE_PLAN.md`.
  - **Verified:** test counts above. Packaged agent on this Mac is unsupported
    (live probe); preview uses sample Linux data.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO remain **OPEN**.
  - **Still paper:** hypervisor binary fetch (phase 19); guest image fetch;
    launch config; VFIO bind; pairwise ACLs; live guest Tailscale/rclone;
    FR13 `latency_ms`.
- Flagged for the next agent:
  - Start at **phase 19**. Do not invoke Firecracker/QEMU. Do not mark gates
    closed. Do not create `gate-*-result.md`.
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; still ahead of
  `origin/main`; did not push.
