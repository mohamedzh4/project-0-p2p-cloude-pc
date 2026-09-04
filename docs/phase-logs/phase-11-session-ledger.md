# Handoff — phase 11 session history + internal credits ledger (2026-09-04)

- Phase goal: FR7 + FR16 — meter session duration into a Postgres ledger
  (user debit / host credit). No payment processor.
- Verified git state at start: `79c250e` on `main`, working tree clean, branch
  ahead of `origin/main` by 11. Did not push.
- Files created/changed:
  - `matchmaker/alembic/versions/0004_ledger_entries.py`
  - `matchmaker/src/matchmaker/ledger.py`
  - `matchmaker/src/matchmaker/models.py`, `config.py`, `schemas.py`
  - `matchmaker/src/matchmaker/routers/sessions.py`, `routers/hosts.py`
  - `matchmaker/tests/test_ledger.py`, `tests/test_config.py`, `tests/conftest.py`
  - `matchmaker/.env.example`, `matchmaker/README.md`
  - `host-agent/src-tauri/src/agent_status.rs`, `matchmaker.rs`, `commands.rs`,
    `tray.rs`
  - `host-agent/src/App.tsx`, `api.ts`
  - `host-agent/README.md`, `host-agent/.env.example`
  - `web/src/App.tsx`, `api.ts`, `types.ts`, `api.test.ts`, `styles.css`
  - `web/README.md`
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-11-session-ledger.md` (this file)
- Key decisions made and why:
  - **Internal meter + ledger only.** Human chose no Stripe/PayPal. Credits are
    integer rows, not a “paid” card-charge flag.
  - **Rate:** required env `CREDITS_PER_MINUTE` (non-negative int). Fail-fast if
    missing or not an integer. `0` is allowed (free sessions, rows still written).
    Pytest setdefaults to `1` when unset.
  - **Metering rule:** on `POST /v1/sessions/{id}/end`, duration is
    `max(0, floor((ended_at - started_at).total_seconds()))`. Billed minutes
    round up (`(seconds + 59) // 60`); a zero-length session is 0 minutes.
    Amount = billed minutes × `CREDITS_PER_MINUTE`. Clock is occupancy
    `startedAt`, not “desktop ready.”
  - **Schema:** `ledger_entries` with unique `(session_id, party)` so a second
    end (or concurrent end) cannot double-credit. User row: `party=user`,
    `direction=debit`. Host row: `party=host`, `direction=credit`. Same amount
    both sides. Stores `duration_seconds` and `rate_credits_per_minute`.
  - **Endpoints:** `GET /v1/sessions` (user JWT) → `{ sessions, creditsDebited }`.
    `GET /v1/hosts/{id}/earnings` (bootstrap Bearer) → `{ hostId, creditsAccrued }`.
    End response adds `creditsDebited`. JSON camelCase.
  - **Host Agent:** Rust `GET` earnings; webview still does not `fetch` the
    Matchmaker. Copy is `{n} credits accrued (not paid out)`. Unreachable
    Matchmaker → `earnings unavailable`. Not registered → `0`. Tray tooltip
    stays presence-only (no extra HTTP).
  - **Did not** change HostSelector signature, occupancy indexes, GPU
    exclusivity, CSP, `freezePrototype`, Argon2id, or JWT-from-env. Session
    router still does not import `RankedHostSelector` or name hypervisors.
- Test results:
  - Matchmaker: **94 passed**, 0 failed. Was 81. Added fail-fast rate env,
    billed-minute math, ending a session writes two ledger rows, double end
    and concurrent end stay two rows, concurrent booking still 201+409
    (existing), history lists owner sessions, host earnings accrued.
  - Host Agent: **93 passed**, 0 failed (`cargo test` in `src-tauri`). Was 89.
    Added accrued earnings JSON, GET earnings, zero when unregistered, fail-fast
    without Matchmaker URL. `cargo clippy -- -D warnings` clean.
    `npm run typecheck` clean.
  - Web: **27 passed**, 0 failed (`vitest run`). Was 26. Added `listSessions`
    camelCase. `npm run typecheck` clean.
- What’s built / verified / needs hardware / assumptions:
  - **Built:** duration meter at session end, ledger table, user history,
    host earnings endpoint, Host Agent earnings copy, harness history panel.
  - **Verified:** pytest/cargo/vitest counts above. Host Agent preview on
    `:1420`: idle / in-session / paused; earnings stay
    `0 credits accrued (not paid out)`. Live book/end against the long-running
    uvicorn on `:8000` is still the pre-ledger process (GET `/v1/sessions`
    returned 405). Restart after migrate.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO unchanged.
  - **Assumptions:** `CREDITS_PER_MINUTE` is operator-set; tests use 1 when
    the env is empty. Real payouts are out of scope.
- Flagged for the next agent:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** No `gate-*-result.md`.
  - Next software phase is **12** (heartbeat-silence teardown + re-match).
    Do not start 14–15 without asking (accounts).
  - Restart Matchmaker and `alembic upgrade head` so `0004_ledger_entries`
    and `CREDITS_PER_MINUTE` are live. Do not commit `.env`.
  - Aikido MCP `namespaceStatus: error`; not a scan pass. Run `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; working tree should
  be clean; still ahead of `origin/main`; did not push.
