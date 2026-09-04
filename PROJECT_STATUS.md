# Project Status — last updated: 2026-09-04, after phase-plan inventory

## Done

- Specs: architecture + cahier des charges in `docs/` (OS-conditional Standard
  Lab, GPU exclusivity, four planes). On `main`.
- Host Agent v0 (`host-agent/`, on `main`): CPU/RAM via sysinfo; NVIDIA GPUs via
  nvml-wrapper (Windows/Linux, graceful empty otherwise); sliders + GPU picker
  with exclusivity copy; config v2 (`sharedGpuId`); CSP, `freezePrototype`,
  capabilities off `core:default`. **30/30 Rust tests passed** on 2026-09-04.
- Matchmaker Phase-1 slice (`matchmaker/`, **working tree only — not on
  `origin/main`**): Argon2id + JWT; signup/login/me; occupancy via partial unique
  indexes; `HostSelector` / `SingleApprovedHostSelector`; seed host;
  camelCase session API. **23/23 pytest passed** on 2026-09-04.
- Web harness (`web/`, **working tree only — not on `origin/main`**): signup /
  login, request session, occupied/existing/ended copy, open live Kasm in a new
  tab. **`npm run typecheck` passed**; no automated tests yet.

## In Progress

- Planning session complete. No feature phase started. Next agent owns **phase 1**
  (lock Matchmaker + web on `main`, add web tests).

## Up Next

1. Phase 1 — Lock the control-plane harness on `main` (commit Matchmaker + web,
   add web tests, keep 23 pytest green).
2. Phase 2 — Host inventory registration on the Matchmaker (pending → manual
   approve; do not change `HostSelector`).
3. Phase 3 — Host Agent Rust client registering v2 caps (no webview `fetch`).

## Known Risks / Open Items

- **GATE-FC / GATE-WHPX / GATE-PRO:** Firecracker+Kasm, QEMU+WHPX+Kasm, and one
  Pro GPU session are **not evidenced in this repo**. No runbook, guest image,
  or live `SEED_HOST_KASM_URL` (example seed is `https://example.invalid/kasm`).
  Do **not** mark these done. This Mac cannot host (no KVM/WHPX path).
- Matchmaker and `web/` exist locally but are uncommitted; `origin/main` is still
  Host Agent + specs only (`5425ccf`).
- No Host Agent ↔ Matchmaker calls, no heartbeats, no VM provision, no
  Tailscale/Headscale, no Storj, no billing, no intent capture (FR2).
- Guest egress via platform exit nodes is still on paper (legal exposure at
  public launch — architecture §9 item 5).
- Aikido MCP has been unreliable here; do not treat a missing scan as a pass.
- Consumer GPU EULAs and ToS/liability still need legal review before commercial
  launch (architecture §9 items 10–11).

## Notes for next agent/session

- You are starting **phase 1**. Read `PHASE_PLAN.md` and this file; do not start
  phase 2 in the same session.
- Local secrets already exist (`matchmaker/.env`, `web/.env`) — never commit them.
  Pytest needs `TEST_DATABASE_URL` and `JWT_SECRET`.
- `HostSelector.select(db, *, lab_tier) -> Host | None` is the matching boundary.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.” A prior draft of
  that file overclaimed Phase 0 as done off-Mac; that overclaim was corrected in
  the planning commit.
