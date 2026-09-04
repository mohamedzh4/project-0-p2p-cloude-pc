# Handoff — phase 4 heartbeats (2026-09-04)

- Phase goal: Host Agent pings the Matchmaker from Rust; selector skips stale
  hosts; occupancy stays on session indexes, not a host flag.
- Verified git state at start: `df7067f` on `main`, working tree clean, branch
  ahead of `origin/main` by 4. Did not push.
- Files created/changed:
  - `matchmaker/alembic/versions/0002_host_last_seen.py` (additive `last_seen_at`)
  - `matchmaker/src/matchmaker/models.py`, `schemas.py`, `inventory.py`,
    `matching.py`, `seed.py`, `routers/hosts.py`
  - `matchmaker/tests/conftest.py`, `helpers.py`, `test_matching.py`,
    `test_hosts.py`, `test_sessions.py`
  - `matchmaker/README.md`
  - `host-agent/src-tauri/src/matchmaker.rs`, `lib.rs`, `commands.rs`
  - `host-agent/src/App.tsx`, `styles.css`
  - `host-agent/.env.example`, `README.md`
  - `PHASE_PLAN.md` (contracts: heartbeat path + `lastSeenAt`)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/phase-logs/phase-4-heartbeats.md` (this file)
- Key decisions made and why:
  - Stale threshold is **45 seconds** (`HEARTBEAT_STALE_AFTER`). Architecture
    §10 says ping every few seconds and treat ~30–60s of silence as stale; 45s
    is the midpoint. Host Agent pings every **10s**, so a few missed pings are
    tolerated without marking the host dead.
  - Never-seen hosts (`last_seen_at` null) are not selectable. Liveness is the
    ping, not register/update (those do not stamp last-seen).
  - Staleness is applied in `SingleApprovedHostSelector` via
    `heartbeat_freshness_cutoff()` (SQL `last_seen_at >= cutoff`). Session
    router still only calls `selector.select(db, lab_tier=...)`.
  - `HostSelector.select(db, *, lab_tier) -> Host | None` unchanged.
    `SingleApprovedHostSelector` imported only from `matching.py` / `deps.py`
    (plus `tests/test_matching.py`).
  - Occupancy remains partial unique indexes. Heartbeat does not approve, change
    caps, or free an occupied host. Active sessions are **not** interrupted when
    pings stop (deferred).
  - Pause = local `sharingEnabled=false` skips pings so the host goes stale.
    Not a Matchmaker field.
  - Heartbeat HTTP stays in Rust (`POST /v1/hosts/{id}/heartbeat`, empty JSON
    `{}`, bootstrap Bearer, redirects still disabled). Background thread in
    `lib.rs`; best-effort ping after a successful register. CSP unchanged.
  - Reused `MATCHMAKER_URL` + `HOST_BOOTSTRAP_TOKEN`. No new required secrets.
    Fail-fast when a ping would send and those vars are missing. Paused / not
    registered skips without requiring env.
  - Seed stamps `last_seen_at` so the local harness can book without a real
    agent; after 45s of silence the seed host is stale too (re-seed or ping).
    Test fixtures stamp a fresh last-seen so the original booking cases still
    pass.
  - Additive Alembic only. Did not rewrite `0001_initial`.
  - Did not provision, Firecracker/QEMU, Tailscale, auto-approve, or add a host
    occupancy column. GPU exclusivity / OS-conditional backend untouched.
- Test results:
  - Matchmaker: **52 passed**, 0 failed (`pytest`). Was 37. Added cutoff helper
    tests, never-seen/stale/fresh selector cases, pending+fresh still skipped,
    heartbeat auth/404/no-approve, inventory PUT is not a ping, heartbeat does
    not free occupancy, stale/never-seen booking 409, heartbeat then 201.
    Concurrent double-booking still **201+409**. Original 37 still pass
    (fixtures now include a fresh last-seen).
  - Host Agent: **52 passed**, 0 failed (`cargo test`). Was 46. Added skip
    (no config / paused / no host id), POST URL + redacted bearer, fail-fast
    env when sending, 404 rejected. `cargo clippy -- -D warnings` clean.
    `npm run typecheck` clean. Browser preview on :1420: save with sharing on
    shows the heartbeat copy; GPU exclusivity copy still appears when
    `shareGpu` is on. Preview still simulates registration (no real ping).
  - Web suite not re-run (unchanged this session).
- New API / fields / env:
  - `POST /v1/hosts/{id}/heartbeat` (host bootstrap Bearer) → 200 HostResponse
  - Host JSON: `lastSeenAt` (null until a ping)
  - DB: `hosts.last_seen_at` (nullable timestamptz)
  - No new env vars. Host Agent still `MATCHMAKER_URL` + `HOST_BOOTSTRAP_TOKEN`
    (+ optional `HOST_DISPLAY_NAME`)
- Flagged for the orchestrator:
  - Hardware gates OPEN. Runbooks untouched; no `gate-*-result.md`; seed URL
    still `https://example.invalid/kasm`.
  - Heartbeat silence does not end an active session.
  - Shared bootstrap token can still update or ping any host id that knows it.
  - Aikido MCP `namespaceStatus: error` / no usable `aikido_full_scan`; not a
    scan pass.
  - `origin/main` still behind local `main` (planning + phases 1–4). Nothing
    pushed.
  - Next session must start at **phase 5** (OS-conditional provisioner stubs)
    and stop there.
