# Project Status — last updated: 2026-09-04, after phase 1 (lock harness)

## Done

- Specs: architecture + cahier des charges in `docs/` (OS-conditional Standard
  Lab, GPU exclusivity, four planes). On `main`.
- Host Agent v0 (`host-agent/`, on `main`): CPU/RAM via sysinfo; NVIDIA GPUs via
  nvml-wrapper (Windows/Linux, graceful empty otherwise); sliders + GPU picker
  with exclusivity copy; config v2 (`sharedGpuId`); CSP, `freezePrototype`,
  capabilities off `core:default`. **30/30 Rust tests passed** on 2026-09-04
  (planning session; not re-run in phase 1).
- Matchmaker Phase-1 slice (`matchmaker/`, **on local `main`**, not yet pushed):
  Argon2id + JWT-from-env; signup/login/me; occupancy via partial unique indexes
  `sessions_one_active_per_host` / `sessions_one_active_per_user`; `HostSelector`
  / `SingleApprovedHostSelector` via `get_host_selector()`; seed host; camelCase
  session API. **23/23 pytest passed** on 2026-09-04 (phase 1 re-run).
- Web harness (`web/`, **on local `main`**, not yet pushed): signup / login,
  request session, occupied/existing/ended copy, open live Kasm in a new tab.
  Vitest covers `isLiveKasmUrl`, occupied 409, existing-session 200, and API
  error mapping. **16/16 vitest passed**; **`npm run typecheck` passed**.
- Hardware-gate runbooks (gates remain **OPEN**): `docs/runbooks/gate-fc.md`,
  `docs/runbooks/gate-whpx.md`, `docs/runbooks/gate-pro.md`.

## In Progress

- Cahier Phase 1 software (MVP Standard Lab, hosts approved manually). Execution
  phase 1 is committed locally. Next agent owns **phase 2** (host inventory
  registration). Do not start phase 2 in a session that is still wrapping phase 1.

## Up Next

1. Phase 2 — Host inventory registration on the Matchmaker (pending → manual
   approve; do not change `HostSelector`).
2. Phase 3 — Host Agent Rust client registering v2 caps (no webview `fetch`).
3. Phase 4 — Heartbeats (staleness through `HostSelector` only).

## Known Risks / Open Items

- **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** Runbooks exist; there is no
  human result file, guest image, hypervisor script, or live
  `SEED_HOST_KASM_URL` (seed example is still `https://example.invalid/kasm`).
  This Mac cannot host. Do **not** mark these done. Closing a gate requires the
  runbook result file **and** an explicit human confirmation.
- Local `main` is ahead of `origin/main` (planning commit + this phase 1 commit).
  Nothing was pushed this session.
- No Host Agent ↔ Matchmaker calls, no heartbeats, no VM provision, no
  Tailscale/Headscale, no Storj, no billing, no intent capture (FR2).
- Guest egress via platform exit nodes is still on paper (legal exposure at
  public launch — architecture §9 item 5).
- Aikido MCP was unavailable this session (`namespaceStatus: error`); do not
  treat a missing scan as a pass.
- Consumer GPU EULAs and ToS/liability still need legal review before commercial
  launch (architecture §9 items 10–11).
- Step 0 note: `PROJECT_CONTEXT.md` still listed `git log` as two commits after
  the planning commit (`14d6448`). That snapshot was stale, not a hardware-gate
  overclaim; it is corrected in this update. Hardware gates were already recorded
  as open and were not inflated.

## Notes for next agent/session

- You are starting **phase 2**. Read `PHASE_PLAN.md`, this file, and
  `docs/phase-logs/phase-1-lock-harness.md`. Do not start phase 3 in the same
  session.
- Local secrets already exist (`matchmaker/.env`, `web/.env`) — never commit them.
  Pytest needs `TEST_DATABASE_URL` and `JWT_SECRET` (compose + existing `.env`
  worked on 2026-09-04; 23 passed without changing compose).
- `HostSelector.select(db, *, lab_tier) -> Host | None` is the matching boundary.
  Do not add Matchmaker API fields unless the phase 2 handoff documents them.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.”
