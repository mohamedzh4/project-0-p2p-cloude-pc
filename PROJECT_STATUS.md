# Project Status — last updated: 2026-09-04, after phase 2 (host registration)

## Done

- Specs: architecture + cahier des charges in `docs/` (OS-conditional Standard
  Lab, GPU exclusivity, four planes). On `main`.
- Host Agent v0 (`host-agent/`, on `main`): CPU/RAM via sysinfo; NVIDIA GPUs via
  nvml-wrapper (Windows/Linux, graceful empty otherwise); sliders + GPU picker
  with exclusivity copy; config v2 (`sharedGpuId`); CSP, `freezePrototype`,
  capabilities off `core:default`. **30/30 Rust tests passed** on 2026-09-04
  (planning session; not re-run in phase 2).
- Matchmaker (`matchmaker/`, **on local `main`**, not yet pushed): Argon2id +
  JWT-from-env; signup/login/me; host register/update of declared caps; new
  hosts **pending**; manual approve (`POST /v1/hosts/{id}/approve` or
  `python -m matchmaker.seed --approve-host-id`); occupancy via partial unique
  indexes `sessions_one_active_per_host` / `sessions_one_active_per_user`;
  `HostSelector` / `SingleApprovedHostSelector` via `get_host_selector()`
  (still `status == approved` only); seed upserts by display name; camelCase
  JSON. **37/37 pytest passed** on 2026-09-04 (phase 2; original 23 plus
  inventory cases). Concurrent booking still 201+409.
- Web harness (`web/`, **on local `main`**, not yet pushed): signup / login,
  request session, occupied/existing/ended copy, open live Kasm in a new tab.
  **16/16 vitest** and **`npm run typecheck`** last run in phase 1 (not re-run
  in phase 2; web unchanged).
- Hardware-gate runbooks (gates remain **OPEN**): `docs/runbooks/gate-fc.md`,
  `docs/runbooks/gate-whpx.md`, `docs/runbooks/gate-pro.md`.

## In Progress

- Cahier Phase 1 software (MVP Standard Lab, hosts approved manually). Execution
  phase 2 is committed locally. Next agent owns **phase 3** (Host Agent Rust
  client registering v2 caps). Do not start phase 3 in a session that is still
  wrapping phase 2.

## Up Next

1. Phase 3 — Host Agent Rust client registering clamped v2 caps with the
   Matchmaker phase 2 API (no webview `fetch`).
2. Phase 4 — Heartbeats (staleness through `HostSelector` only).
3. Phase 5 — OS-conditional provisioner stubs.

## Known Risks / Open Items

- **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** Runbooks exist; there is no
  human result file, guest image, hypervisor script, or live
  `SEED_HOST_KASM_URL` (seed example is still `https://example.invalid/kasm`).
  This Mac cannot host. Do **not** mark these done. Closing a gate requires the
  runbook result file **and** an explicit human confirmation.
- Local `main` is ahead of `origin/main` (planning + phase 1 + this phase 2
  commit). Nothing was pushed this session.
- Host register uses a shared `HOST_BOOTSTRAP_TOKEN`; anyone with that token
  can update a known host id. Per-host tokens are not in this phase.
- Existing local `matchmaker/.env` did not have the new host tokens. Pytest
  setdefaults when they are missing; **uvicorn will fail until**
  `HOST_BOOTSTRAP_TOKEN` and `HOST_APPROVE_TOKEN` (different values) are added
  from `.env.example`. Do not commit `.env`.
- No Host Agent ↔ Matchmaker calls yet, no heartbeats, no VM provision, no
  Tailscale/Headscale, no Storj, no billing, no intent capture (FR2).
- Guest egress via platform exit nodes is still on paper (legal exposure at
  public launch — architecture §9 item 5).
- Aikido MCP was unavailable this session (`namespaceStatus: error`); do not
  treat a missing scan as a pass.
- Consumer GPU EULAs and ToS/liability still need legal review before commercial
  launch (architecture §9 items 10–11).

## Notes for next agent/session

- You are starting **phase 3**. Read `PHASE_PLAN.md`, this file, and
  `docs/phase-logs/phase-2-host-registration.md`. Do not start phase 4 in the
  same session.
- Phase 2 API for the Rust client: `POST /v1/hosts` or `PUT /v1/hosts/{id}`
  with `Authorization: Bearer $HOST_BOOTSTRAP_TOKEN`. Body is camelCase
  declared caps. Do **not** send `HOST_APPROVE_TOKEN` from the agent. Fail if
  Matchmaker URL / host bootstrap token (and a stable host id if using PUT)
  are missing. Webview still must not `fetch` the Matchmaker.
- `HostSelector.select(db, *, lab_tier) -> Host | None` is unchanged. Do not
  import `SingleApprovedHostSelector` outside `matching.py` / `deps.py`.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.”
