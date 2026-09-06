# Handoff — phase 23 insufficient-resource rejection UX (2026-09-06)

- Phase goal: If this PC cannot host a tier, tell the volunteer **why** in
  the Host Agent window, do not offer that tier, and do not half-provision.
- Verified git state at start: `fcab0a8` on local `main`, working tree clean,
  ahead of `origin/main`. Did not invent hardware logs. Did not create
  `gate-*-result.md`. `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.
  Did not push. Did not boot a VM.
- Files created/changed:
  - `host-agent/src-tauri/src/eligibility.rs` (new; no Tauri)
  - `host-agent/src-tauri/src/disk_space.rs` (new; spare bytes on image-cache volume)
  - `host-agent/src-tauri/src/launch.rs`, `provision_http.rs`, `commands.rs`, `lib.rs`
  - `host-agent/src-tauri/Cargo.toml` (sysinfo `disk` feature for the spare-disk probe only)
  - `host-agent/src/types.ts`, `api.ts`, `App.tsx`
  - `host-agent/src/components/CapabilityPanel.tsx`
  - `host-agent/README.md`
  - `docs/runbooks/gate-fc.md`, `gate-whpx.md`, `gate-pro.md` (Host Agent paste steps)
  - `PHASE_PLAN.md` (phase 23 done; last numbered automation phase)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-23-insufficient-resource-ux.md` (this file)
- Key decisions made and why:
  - **One offer, not three cards arguing.** Probe (phase 18) stays hardware-only
    so hypervisor/image fetch still run when sliders are low. `eligibility.rs`
    combines probe + clamped sliders + optional spare disk + a refused VFIO
    plan. The Standard/Pro cards use that combined offer.
  - **Documented floor.** 2 vCPU / 4 GiB (already in launch config) plus 50 GiB
    spare disk (Kasm single-server). Unknown disk does not block. A machine
    that cannot spare the floor after clamp is never offered.
  - **Do not offer; do not half-provision.** Launch with a probe does not emit
    a config (`TierNotOffered`). Live `POST /v1/provision` takes a
    `ProvisionGate` (probe + hardware + optional sharing + disk) and returns
    400 with `detail` — no stub URL, occupancy unset. Existing ungated HTTP
    tests still cover the stub planner.
  - **Pro opt-in is not a hardware block.** shareGpu off still *offers* Pro
    with exclusivity copy. A refused VFIO plan *with* shareGpu on blocks Pro.
  - **GPU exclusivity copy stays** on the Pro offer, the GPU picker, and Pro
    launch `{ exclusive: true }` when the tier is offered.
  - **Last automation phase.** Runbooks tell a human what to run on
    Linux/Windows metal and what to paste. Gates stay OPEN. Leftover **19b**
    / **22b** stay paper. `HostSelector.select`, occupancy indexes, CSP,
    Argon2id, JWT unchanged. Never invokes a hypervisor.
- Test results:
  - Host Agent: **231 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 208. Added Mac / Linux-without-KVM / no second display / no IOMMU /
    insufficient vCPU, RAM, disk / machine too small / refused VFIO blocks
    Pro / shareGpu off still offers Pro / launch does not emit / provision
    gate 400 without occupancy.
  - Matchmaker: **161 passed**, 0 failed. Unchanged.
  - Web: **31 passed**, 0 failed. Host Agent `npm run typecheck` passed.
- What’s built / verified / needs hardware / still paper:
  - **Built:** combined Standard/Pro offer in the window; gated live provision.
  - **Verified:** test counts above. Browser preview on `localhost:1420`
    (sample Linux dual-GPU): both tiers Available; lowering CPU below 2 flips
    Standard/Pro to Not available with the vCPU reason; dedicating a GPU keeps
    exclusivity copy.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO remain **OPEN**.
  - **Still paper:** live VFIO on metal; Windows DDA apply (**22b**); Windows
    QEMU silent install (**19b**); pairwise ACLs; live guest Tailscale/rclone;
    FR13 `latency_ms`. No numbered phase 24.
- Flagged for the next agent:
  - There is **no phase 24**. Close gates only from real metal using the
    Host Agent runbook paste steps. Do not invent `gate-*-result.md`. Do not
    invoke Firecracker/QEMU. Do not spawn PowerShell (**22b**). Do not mark
    gates closed from this Mac.
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; still ahead of
  `origin/main`; did not push.
