# Handoff — phase 2 host registration (2026-09-04)

- Phase goal: Let hosts declare caps on the Matchmaker; new rows start pending
  so `HostSelector` still only books manually approved hosts.
- Files created/changed:
  - `matchmaker/src/matchmaker/routers/hosts.py` (register / update / approve)
  - `matchmaker/src/matchmaker/inventory.py` (pending create, cap apply, approve)
  - `matchmaker/src/matchmaker/schemas.py` (`HostInventoryRequest` / `HostResponse`)
  - `matchmaker/src/matchmaker/config.py` (`HOST_BOOTSTRAP_TOKEN`, `HOST_APPROVE_TOKEN`)
  - `matchmaker/src/matchmaker/deps.py` (static Bearer checks; selector unchanged)
  - `matchmaker/src/matchmaker/models.py` (`HostStatus.PENDING`)
  - `matchmaker/src/matchmaker/seed.py` (upsert by display name; `--approve-host-id`)
  - `matchmaker/src/matchmaker/main.py`, `matching.py`
  - `matchmaker/tests/test_hosts.py`, `conftest.py`, `helpers.py`, `test_matching.py`
  - `matchmaker/.env.example`, `matchmaker/README.md`
  - `PHASE_PLAN.md` (contracts table)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/phase-logs/phase-2-host-registration.md` (this file)
- Key decisions made and why:
  - No Alembic revision: `status` is already text; omitted `kasmUrl` is stored
    as `""` and returned as `null`. Additive columns were not required.
  - New hosts are always `pending`. Update never flips status. Approve uses a
    separate env token so the Host Agent bootstrap secret cannot self-approve.
  - `PUT /v1/hosts/{id}` upserts (201 if the id is new) so a later Rust client
    can keep a stable host id. `POST /v1/hosts` still allocates a uuid.
  - Seed upserts by `SEED_HOST_DISPLAY_NAME` instead of `SELECT … LIMIT 1`, so
    a registered pending host is not overwritten into approved.
  - GPU: `shareGpu=true` requires `sharedGpuId`; the id still round-trips when
    sharing is off. Copy says exclusive, not a concurrent slice.
  - Occupancy still partial unique indexes. `HostSelector.select(db, *, lab_tier)`
    signature unchanged. `SingleApprovedHostSelector` imported only from
    `matching.py` / `deps.py` (plus `tests/test_matching.py`).
  - Did not call the Host Agent, add heartbeats, provision, or Tailscale.
  - Did not write `matchmaker/.env`; operator must add the two host tokens
    locally (pytest setdefaults only when they are missing).
- Test results:
  - Matchmaker: **37 passed**, 0 failed (`pytest`). Original 23 still pass.
    Added pending-vs-approved (API + selector), GPU id round-trip on
    register/update, no auto-approve, seed isolation, approve-then-book.
    Concurrent double-booking still **201+409**.
  - Web / Host Agent suites not re-run (unchanged this session).
- Flagged for the orchestrator:
  - Hardware gates OPEN. Runbooks untouched; no `gate-*-result.md`; seed URL
    still `https://example.invalid/kasm`.
  - Contract additions: `POST /v1/hosts`, `PUT /v1/hosts/{id}`,
    `POST /v1/hosts/{id}/approve`; host JSON as in `PHASE_PLAN.md`. Env:
    `HOST_BOOTSTRAP_TOKEN` and `HOST_APPROVE_TOKEN` (must differ). Phase 3
    Rust client should send bootstrap Bearer, not the approve token, and
    must not `fetch` from the webview.
  - Shared bootstrap token can update any host id that knows it; per-host
    tokens are deferred.
  - Aikido MCP `namespaceStatus: error`; not a scan pass.
  - `origin/main` still `5425ccf`. Local `main` is planning + phase 1 + this
    commit. Nothing pushed.
  - Next session must start at **phase 3** and stop there.
