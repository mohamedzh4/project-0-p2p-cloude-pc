# Handoff — phase 12 heartbeat-silence teardown + re-match (2026-09-04)

- Phase goal: FR14 remainder + FR15 — after ~45s of host silence, interrupt the
  occupied session, bill the ledger once, and rematch via `HostSelector.select`.
- Verified git state at start: `d7de565` on `main`, working tree clean, branch
  ahead of `origin/main` by 12. Did not push. GATE-FC / GATE-WHPX / GATE-PRO
  remain OPEN.
- Files created/changed:
  - `matchmaker/src/matchmaker/silence.py`
  - `matchmaker/src/matchmaker/main.py`, `config.py`, `models.py`, `matching.py`,
    `provision.py`
  - `matchmaker/src/matchmaker/routers/sessions.py`, `routers/hosts.py`
  - `matchmaker/tests/test_silence.py`, `tests/test_config.py`, `tests/conftest.py`
  - `matchmaker/.env.example`, `matchmaker/README.md`
  - `host-agent/README.md`
  - `web/README.md`
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/phase-logs/phase-12-silence-teardown.md` (this file)
- Key decisions made and why:
  - **Status `interrupted` vs `ended`.** User `POST .../end` stays `ended`.
    Silence teardown uses `interrupted`. Both set `endedAt` and call
    `settle_ended_session`. Partial unique indexes still only treat `status =
    'active'` as occupying — no migration (status is unconstrained text).
    Ending an interrupted session is idempotent and does **not** rename it to
    `ended`. JSON camelCase unchanged; history already returns `status`.
  - **No new HTTP routes.** Reclaim runs on `POST /v1/sessions`,
    `GET /v1/sessions`, and `POST /v1/hosts/{id}/heartbeat` (after stamping
    `lastSeenAt` so a late ping keeps that host's own session), plus a
    FastAPI lifespan loop. `STALE_SESSION_SWEEP_INTERVAL_SECONDS` default 15;
    `0` disables the loop only (pytest sets 0 so TRUNCATE cannot race).
  - **Rematch is one `HostSelector.select(db, *, lab_tier)`** using the
    interrupted host's `labTier`. `silence.py` / session / host routers do not
    import `RankedHostSelector` and do not name hypervisors. Signature
    unchanged. If select returns None, the user has no active session; the next
    book is 409 until a fresh approved host exists. Concurrent book stays
    201+409 (no runner-up retry on IntegrityError).
  - **Reclaim before a new book.** A displaced user can take the last remaining
    live host; the new booker then 409s. Documented, not a lock.
  - **Ledger.** Same meter as user end. Unique `(session_id, party)` plus
    `settle_ended_session` existing-debit check. Second sweep / concurrent GET
    does not double-bill.
  - **Pause.** Still no Matchmaker pause field. Skipping heartbeats is
    indistinguishable from disconnect; after 45s an occupied paused host is
    interrupted.
  - **Host Agent local occupancy** is not cleared (no teardown port). Left as
    a follow-up, not phase 12b — rematch fit in this session.
  - **Did not** change occupancy indexes, GPU exclusivity, CSP,
    `freezePrototype`, Argon2id, JWT-from-env, Tailscale, Storj, or Stripe.
- Test results:
  - Matchmaker: **105 passed**, 0 failed. Was 94. Added silence interrupt +
    single ledger write, second sweep / end-after-interrupt no double-bill,
    rematch onto a second fresh host (200 existing after reclaim), rematch
    none → 409, live heartbeat does not interrupt its own session, heartbeat
    of a peer reclaims a stale occupied host, concurrent reclaim stays two
    ledger rows, sweep-interval env fail-fast, source guard (no concrete
    selector / hypervisor names). Existing concurrent booking still 201+409.
  - Host Agent: **93 passed**, 0 failed (`cargo test` in `src-tauri`). Unchanged.
  - Web: **27 passed**, 0 failed (`vitest run`). `npm run typecheck` clean.
    History already prints `item.status`.
- What’s built / verified / needs hardware / assumptions:
  - **Built:** occupied silent host → `interrupted` + ledger; rematch via
    selector; background sweep (disabled in pytest).
  - **Verified:** pytest/cargo/vitest counts above.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO unchanged.
  - **Assumptions:** 45s threshold unchanged. Rematch does not move guest
    files (Storj is phase 15).
- Flagged for the next agent:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** No `gate-*-result.md`.
  - Next software phase is **13** (signed base-image pipeline). Do not start
    14–15 without asking (accounts).
  - Restart Matchmaker so the lifespan sweep is live. Do not commit `.env`.
  - Aikido MCP `namespaceStatus: error`; not a scan pass. Run `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; working tree should
  be clean; still ahead of `origin/main`; did not push.
