# Handoff — phase-plan inventory (2026-09-04)

- Phase goal: Reconstruct real repo state from docs, code, tests, and git; write
  `PHASE_PLAN.md` and the first `PROJECT_STATUS.md`; do not start feature work.
- Files created/changed:
  - `PHASE_PLAN.md` (new)
  - `PROJECT_STATUS.md` (new)
  - `docs/phase-logs/phase-plan-inventory.md` (this file)
  - `PROJECT_CONTEXT.md` (pointer to living status; Phase 0 overclaim removed)
  - `.gitignore` (Python/venv entries already in the working tree, included so
    phase 1 can land `matchmaker/` safely)
- Key decisions made and why:
  - Execution phases are agent sessions for the **Standard Lab MVP software
    loop**, not a rename of cahier 0–4.
  - Cahier Phase 0 is a **hardware gate** (GATE-FC / GATE-WHPX / GATE-PRO), not
    an agent phase, because this Mac cannot host and the repo has no Firecracker,
    QEMU, Kasm, or live session URL.
  - Did **not** treat an uncommitted `PROJECT_CONTEXT.md` line (“Phase 0 done
    off-Mac”) as fact: no runbook, no scripts, seed URL still
    `https://example.invalid/kasm`.
  - Did **not** commit `matchmaker/` or `web/` in this session — that is phase 1,
    after web tests exist.
  - Deferred Tailscale/Storj/billing/Pro until after the MVP orchestrator pass;
    those need accounts or are cahier 2–4.
- Test results (verification only; no tests added):
  - Host Agent: **30 passed**, 0 failed (`cargo test` in `host-agent/src-tauri`)
  - Matchmaker: **23 passed**, 0 failed (`pytest`), 1 Starlette/httpx deprecation
    warning
  - Web: `npm run typecheck` passed; **no test runner / no tests**
- Flagged for the orchestrator:
  - `origin/main` is 2 commits (specs + Host Agent). Matchmaker + web are local
    only.
  - HostSelector contract unchanged; no API fields added this session.
  - Granola MCP was unauthenticated; meeting history was not checked.
  - Next session must start at **phase 1** and stop there.
