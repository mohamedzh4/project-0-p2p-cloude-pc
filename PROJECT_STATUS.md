# Project Status — last updated: 2026-09-04, after phase 5 (provisioner stubs)

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
  While sharing is on, a Rust thread pings `POST /v1/hosts/{id}/heartbeat`
  every 10s. Pause sharing stops pings. Webview still does not `fetch` the
  Matchmaker. **OS-conditional provisioner stubs** in
  `src-tauri/src/provision.rs`: Linux Standard → Firecracker, Windows Standard
  → QEMU+WHPX, Linux Pro → QEMU+VFIO, Windows Pro → Hyper-V GPU assignment;
  macOS is an explicit unsupported variant (not a Firecracker fallback). Pro
  requires `sharedGpuId` and sets `gpuExclusive` plus an exclusivity warning.
  No hypervisor is invoked. **66/66 Rust tests passed** and `npm run typecheck`
  on 2026-09-04 (phase 5). Clippy `-D warnings` clean.
- Matchmaker (`matchmaker/`, **on local `main`**, not yet pushed): Argon2id +
  JWT-from-env; signup/login/me; host register/update of declared caps; new
  hosts **pending**; manual approve (`POST /v1/hosts/{id}/approve` or
  `python -m matchmaker.seed --approve-host-id`); heartbeats stamp
  `hosts.last_seen_at` (`POST /v1/hosts/{id}/heartbeat`); occupancy via partial
  unique indexes `sessions_one_active_per_host` / `sessions_one_active_per_user`;
  `HostSelector` / `SingleApprovedHostSelector` via `get_host_selector()`
  (`status == approved`, fresh heartbeat, not occupied). Additive Alembic
  `0002_host_last_seen`. camelCase JSON including `lastSeenAt`. Session router
  still does not call provision (comment points at `provision::plan`). **52/52
  pytest passed** on 2026-09-04. Concurrent booking still 201+409.
- Web harness (`web/`, **on local `main`**, not yet pushed): signup / login,
  request session, occupied/existing/ended copy, open live Kasm in a new tab.
  **16/16 vitest** and **`npm run typecheck`** last run in phase 1 (not re-run
  in phase 5; web unchanged).
- Hardware-gate runbooks (gates remain **OPEN**): `docs/runbooks/gate-fc.md`,
  `docs/runbooks/gate-whpx.md`, `docs/runbooks/gate-pro.md`.

## In Progress

- Cahier Phase 1 software (MVP Standard Lab, hosts approved manually). Execution
  phase 5 is committed locally. Next agent owns **phase 6** (Matchmaker-triggered
  mock provision). Do not start phase 6 in a session that is still wrapping
  phase 5.

## Up Next

1. Phase 6 — Matchmaker-triggered mock provision.
2. Phase 7 — Orchestrator MVP pass.

## Known Risks / Open Items

- **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** Runbooks exist; there is no
  human result file, guest image, hypervisor script, or live
  `SEED_HOST_KASM_URL` (seed example is still `https://example.invalid/kasm`).
  This Mac cannot host. Do **not** mark these done. Closing a gate requires the
  runbook result file **and** an explicit human confirmation.
- Local `main` is ahead of `origin/main` (planning + phases 1–5). Nothing was
  pushed this session.
- Host register/heartbeat uses a shared `HOST_BOOTSTRAP_TOKEN`; anyone with
  that token can update or ping a known host id. Per-host tokens are not in
  this phase.
- Existing local `matchmaker/.env` must still have `HOST_BOOTSTRAP_TOKEN` and
  `HOST_APPROVE_TOKEN` (different values). Host machines need `MATCHMAKER_URL`
  and `HOST_BOOTSTRAP_TOKEN` (see `host-agent/.env.example`). Do not commit
  `.env`. Do not put `HOST_APPROVE_TOKEN` on the Host Agent.
- Heartbeats mark inventory stale for **new** bookings only. An active session
  is not interrupted when pings stop. Seed stamps `last_seen_at` so the local
  harness can book; after 45s of silence the seed host is also stale (re-seed
  or POST heartbeat).
- Provisioner is stubs only. Matchmaker does not call the Host Agent yet. No
  Tailscale/Headscale, no Storj, no billing, no intent capture (FR2). Local
  `sharingEnabled` is not a Matchmaker field (pause = stop pings).
- Guest egress via platform exit nodes is still on paper (legal exposure at
  public launch — architecture §9 item 5).
- Aikido MCP was unavailable this session (`namespaceStatus: error`); do not
  treat a missing scan as a pass.
- Consumer GPU EULAs and ToS/liability still need legal review before commercial
  launch (architecture §9 items 10–11).

## Notes for next agent/session

- You are starting **phase 6**. Read `PHASE_PLAN.md`, this file, and
  `docs/phase-logs/phase-5-provisioner-stubs.md`. Do not start phase 7 in the
  same session.
- After `HostSelector.select`, call `host_agent_lib::plan_provision` (Rust
  `provision::plan(os, &request) -> Result<ProvisionPlan, ProvisionError>`).
  `connection_url` is `https://provision.invalid/{backend}`. Do not invent a
  second OS selector in Python. Do not put Firecracker/QEMU/WHPX names in
  `sessions.py`. Do not invoke a real hypervisor.
- `HostSelector.select(db, *, lab_tier) -> Host | None` is unchanged. Occupancy
  stays on session indexes. Do not import `SingleApprovedHostSelector` outside
  `matching.py` / `deps.py` (tests may).
- Stale threshold is **45 seconds** (`HEARTBEAT_STALE_AFTER` in `matching.py`).
  Agent ping interval is **10 seconds**. Never-seen (`last_seen_at` null) is
  not selectable.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.”
