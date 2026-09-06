# Handoff — phase 1 lock harness (2026-09-04)

- Phase goal: Commit the local Matchmaker + web harness as the control-plane
  baseline; add web tests; write hardware-gate runbooks; do not start phase 2.
- Files created/changed:
  - `matchmaker/` (new to git: FastAPI, Alembic, tests, compose, `.env.example`)
  - `web/` (new to git: Vite harness, Vitest, `.env.example`)
  - `web/src/sessionRequest.ts` (201/200/409 mapping extracted for tests)
  - `web/src/*.test.ts`, `web/src/components/SessionPanel.test.ts`
  - `docs/runbooks/gate-fc.md`, `gate-whpx.md`, `gate-pro.md`
  - `docs/phase-logs/phase-1-lock-harness.md` (this file)
  - `PHASE_PLAN.md` (gate table now points at runbooks; still OPEN)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md` (stale “2 commits” / “no runbook”
    wording; gates not closed)
- Key decisions made and why:
  - Vitest (`vitest run`, Node environment, mocked `fetch`) — no Postgres for
    the harness suite; pytest remains the API contract.
  - No new Matchmaker API fields; occupancy still partial unique indexes;
    callers still use `get_host_selector()` / `HostSelector`.
  - Runbooks are numbered operator procedures with a result-file template;
    agents still must not close GATE-* without a human.
  - Did not push; did not open a PR; did not start host registration.
- Test results:
  - Matchmaker: **23 passed**, 0 failed (`pytest`), 1 Starlette/httpx
    deprecation warning (unchanged)
  - Web: **16 passed**, 0 failed (`npm test`); `npm run typecheck` passed
  - Host Agent cargo tests not re-run this session (still 30/30 from planning)
- Flagged for the orchestrator:
  - Hardware gates OPEN. Runbooks exist; no `gate-*-result.md`, no live
    `SEED_HOST_KASM_URL`.
  - Aikido MCP `namespaceStatus: error`; auth call timed out. Not a scan pass.
  - Step 0: docs were mostly honest; corrected stale git-log / “no runbook”
    lines. No Phase 0-done overclaim remained in the living status file.
  - `origin/main` is still `5425ccf`. Local `main` is planning + this commit.
  - Next session must start at **phase 2** and stop there.
