# Project Status — last updated: 2026-09-04, after phase 3 (Host Agent client)

## Done

- Specs: architecture + cahier des charges in `docs/` (OS-conditional Standard
  Lab, GPU exclusivity, four planes). On `main`.
- Host Agent (`host-agent/`, **on local `main`**, not yet pushed): CPU/RAM via
  sysinfo; NVIDIA GPUs via nvml-wrapper (Windows/Linux, graceful empty
  otherwise); sliders + GPU picker with exclusivity copy; config v2
  (`sharedGpuId`); CSP, `freezePrototype`, capabilities off `core:default`.
  Rust Matchmaker client registers clamped v2 caps (`POST /v1/hosts` or
  `PUT /v1/hosts/{id}` with bootstrap Bearer); host id persisted in
  `host-identity.json`; GPU exclusivity copy on config and registration.
  **46/46 Rust tests passed** and `npm run typecheck` on 2026-09-04 (phase 3).
  Clippy `-D warnings` clean.
- Matchmaker (`matchmaker/`, **on local `main`**, not yet pushed): Argon2id +
  JWT-from-env; signup/login/me; host register/update of declared caps; new
  hosts **pending**; manual approve (`POST /v1/hosts/{id}/approve` or
  `python -m matchmaker.seed --approve-host-id`); occupancy via partial unique
  indexes `sessions_one_active_per_host` / `sessions_one_active_per_user`;
  `HostSelector` / `SingleApprovedHostSelector` via `get_host_selector()`
  (still `status == approved` only); seed upserts by display name; camelCase
  JSON. **37/37 pytest passed** on 2026-09-04 (re-run in phase 3; unchanged).
  Concurrent booking still 201+409.
- Web harness (`web/`, **on local `main`**, not yet pushed): signup / login,
  request session, occupied/existing/ended copy, open live Kasm in a new tab.
  **16/16 vitest** and **`npm run typecheck`** last run in phase 1 (not re-run
  in phase 3; web unchanged).
- Hardware-gate runbooks (gates remain **OPEN**): `docs/runbooks/gate-fc.md`,
  `docs/runbooks/gate-whpx.md`, `docs/runbooks/gate-pro.md`.

## In Progress

- Cahier Phase 1 software (MVP Standard Lab, hosts approved manually). Execution
  phase 3 is committed locally. Next agent owns **phase 4** (heartbeats). Do not
  start phase 4 in a session that is still wrapping phase 3.

## Up Next

1. Phase 4 — Heartbeats (staleness through `HostSelector` only).
2. Phase 5 — OS-conditional provisioner stubs.
3. Phase 6 — Matchmaker-triggered mock provision.

## Known Risks / Open Items

- **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** Runbooks exist; there is no
  human result file, guest image, hypervisor script, or live
  `SEED_HOST_KASM_URL` (seed example is still `https://example.invalid/kasm`).
  This Mac cannot host. Do **not** mark these done. Closing a gate requires the
  runbook result file **and** an explicit human confirmation.
- Local `main` is ahead of `origin/main` (planning + phases 1–3). Nothing was
  pushed this session.
- Host register uses a shared `HOST_BOOTSTRAP_TOKEN`; anyone with that token
  can update a known host id. Per-host tokens are not in this phase.
- Existing local `matchmaker/.env` must still have `HOST_BOOTSTRAP_TOKEN` and
  `HOST_APPROVE_TOKEN` (different values). Host machines need `MATCHMAKER_URL`
  and `HOST_BOOTSTRAP_TOKEN` (see `host-agent/.env.example`). Do not commit
  `.env`. Do not put `HOST_APPROVE_TOKEN` on the Host Agent.
- No heartbeats, no VM provision, no Tailscale/Headscale, no Storj, no billing,
  no intent capture (FR2). Local `sharingEnabled` is not a Matchmaker field.
- Guest egress via platform exit nodes is still on paper (legal exposure at
  public launch — architecture §9 item 5).
- Aikido MCP was unavailable this session (`namespaceStatus: error`); do not
  treat a missing scan as a pass.
- Consumer GPU EULAs and ToS/liability still need legal review before commercial
  launch (architecture §9 items 10–11).

## Notes for next agent/session

- You are starting **phase 4**. Read `PHASE_PLAN.md`, this file, and
  `docs/phase-logs/phase-3-host-agent-client.md`. Do not start phase 5 in the
  same session.
- Phase 3 client: Rust `ureq` to `POST /v1/hosts` or `PUT /v1/hosts/{id}` with
  `Authorization: Bearer $HOST_BOOTSTRAP_TOKEN`. Caps are re-clamped before
  send. Host id is `host-identity.json`. Webview still must not `fetch` the
  Matchmaker.
- Heartbeats: Matchmaker records last-seen. Selector may ignore stale hosts,
  still through `HostSelector` — do not inline staleness checks in the session
  router. Occupancy stays on session indexes.
- `HostSelector.select(db, *, lab_tier) -> Host | None` is unchanged. Do not
  import `SingleApprovedHostSelector` outside `matching.py` / `deps.py`.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.”
