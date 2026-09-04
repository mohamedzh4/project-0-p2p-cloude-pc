# Handoff — phase 7 orchestrator MVP (2026-09-04)

- Phase goal: Re-run all suites, smoke the control-plane loop, fix harness
  stub-URL drift, document actual MVP vs cahier, extend the plan for remaining
  FRs. Do not close hardware gates. Do not start phase 8.
- Verified git state at start: `3717f75` on `main`, working tree clean, branch
  ahead of `origin/main` by 7. Did not push.
- Files created/changed:
  - `web/src/components/SessionPanel.tsx` + `.test.ts` (`isLiveKasmUrl` rejects
    RFC 2606 `.invalid`, including `provision.invalid`)
  - `web/src/App.tsx` (harness lede)
  - `web/README.md`, `matchmaker/README.md`, `host-agent/README.md`
  - `PHASE_PLAN.md` (FR gap table; execution phases 8–16)
  - `PROJECT_STATUS.md` (rewritten), `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md` (short implemented-vs-paper)
  - `docs/phase-logs/phase-7-orchestrator-mvp.md` (this file)
- Key decisions made and why:
  - No Matchmaker/Host Agent behavior change. Contracts still hold:
    `HostSelector.select(db, *, lab_tier) -> Host | None`; session router does
    not import `SingleApprovedHostSelector`; occupancy is the partial unique
    indexes; config v2 `sharedGpuId` lines up; provision is `HostProvisioner`
    after select; no hypervisor names in `sessions.py`; CSP /
    `freezePrototype` / narrowed capabilities / Argon2id / JWT-from-env
    unchanged.
  - Harness treated `https://provision.invalid/...` as a live desktop (only
    `example.invalid` was filtered). Reject the `.invalid` TLD so mock
    provision cannot show Open desktop.
  - Cahier gaps become numbered phases 8–16. Tailscale (14) and Storj (15)
    stay “ask human first — needs accounts.” GATE-* stay human runbooks.
  - Did not implement FR2, ranking, billing, tray, Tailscale, Storj, Pro
    automation, or image signing. Did not mark GATE-* closed.
- Test results:
  - Host Agent: **77 passed**, 0 failed (`cargo test`). Clippy `-D warnings`
    clean. `npm run typecheck` clean.
  - Matchmaker: **66 passed**, 0 failed (`pytest`). 1 Starlette/httpx
    deprecation warning (unchanged).
  - Web: **16 passed**, 0 failed (`vitest run`); `npm run typecheck` clean.
    Added `.invalid` stub assertions inside the existing `isLiveKasmUrl` case.
- What’s built / verified / needs hardware / assumptions:
  - **Built:** control-plane MVP (auth, pending→approve inventory, 45s
    heartbeats, occupancy indexes, mock provision port, Host Agent v2 caps +
    Rust client + provision stubs).
  - **Verified:** full unit/integration suites; curl smoke on a **fresh**
    Matchmaker (`127.0.0.1:18000`, test DB) + Python `POST /v1/provision`
    stub (`:17842`): pending 409 → approve → never-seen 409 → heartbeat →
    201 stub `kasmUrl` → 200 existing → 409 occupied → end → 201 rebook.
    Harness auth screen still renders (`localhost:5173`, IPv6-only Vite).
  - **Not verified:** live Firecracker/QEMU/Kasm desktop; packaged Host Agent
    listener on this Mac (macOS → `UnsupportedHostOs`); web harness booking
    against the long-running `:8000` process (that process **404s**
    `POST /v1/hosts` — pre-inventory Matchmaker). Playwright signup was not
    driven (credential prompt blocked); placeholder copy covered by vitest.
  - **Needs hardware:** GATE-FC `docs/runbooks/gate-fc.md`, GATE-WHPX
    `docs/runbooks/gate-whpx.md`, GATE-PRO `docs/runbooks/gate-pro.md`.
  - **Assumptions:** smoke bootstrap/approve tokens were process env only, not
    written to `.env`. Pytest still setdefaults those names when missing.
- Flagged for the next agent:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** No `gate-*-result.md`.
  - Next software phase is **8** (FR2 intent capture). Do not start 9–16 in
    that session. Do not start 14–15 without asking the human (accounts).
  - Long-running uvicorn on `:8000` is not the phase-6/7 app. Restart after
    adding `HOST_BOOTSTRAP_TOKEN` / `HOST_APPROVE_TOKEN` to `matchmaker/.env`.
    A phase-7 probe left an active session on that old process’s seed host
    (`849f3421-66b3-4add-ae62-96ee1703c2bd`) until someone ends it or restarts.
  - Aikido MCP `namespaceStatus: error` / no `aikido_full_scan`; not a pass.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; working tree should
  be clean; still ahead of `origin/main`; did not push.
