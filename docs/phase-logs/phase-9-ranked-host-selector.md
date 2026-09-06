# Handoff — phase 9 ranking HostSelector swap (2026-09-04)

- Phase goal: FR3 / FR12 caps slice / FR13 — replace `SingleApprovedHostSelector`
  with `RankedHostSelector` injected via `get_host_selector()`.
- Verified git state at start: `86c7889` on `main`, working tree clean, branch
  ahead of `origin/main` by 9. Did not push.
- Files created/changed:
  - `matchmaker/src/matchmaker/matching.py` (`RankedHostSelector`; remnant
    `SingleApprovedHostSelector` removed)
  - `matchmaker/src/matchmaker/deps.py` (`get_host_selector()` → ranked class)
  - `matchmaker/src/matchmaker/models.py` (`LabTier.PRO`)
  - `matchmaker/src/matchmaker/routers/sessions.py` (plug-in comment only; still
    `HostSelector` via Depends; no concrete selector import)
  - `matchmaker/tests/test_matching.py`, `test_sessions.py`, `conftest.py`
  - `matchmaker/README.md`, `host-agent/README.md`
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-9-ranked-host-selector.md` (this file)
- Key decisions made and why:
  - **Ranking rule** among eligible hosts (approved, matching `lab_tier`, not
    occupied, `last_seen_at` within 45s; never-seen skipped):
    1. `shared_logical_cores` descending
    2. `shared_memory_bytes` descending
    3. `host.id` ascending (deterministic tie-break already on the row)
  - Pro additionally requires `shareGpu` and a non-empty `sharedGpuId`. GPU
    is exclusive for the session, not a concurrent slice. Standard does not
    rank on GPU.
  - No latency or reliability column. PHASE_PLAN said “when those fields
    exist”; they do not, so no fake scores and no additive migration.
  - `HostSelector.select(db, *, lab_tier) -> Host | None` unchanged. Session
    router still does not import the concrete class.
  - Occupancy remains partial unique indexes. Concurrent double-book of one
    host is still 201+409. A race on the winner does **not** fall through to
    the runner-up (re-match is phase 12). Sequential books of two free hosts
    do assign winner then runner-up.
  - `SingleApprovedHostSelector` replaced rather than kept as an alias.
  - Did not start Tailscale/Storj. Did not mark GATE-* closed.
- Test results:
  - Host Agent: **77 passed**, 0 failed (`cargo test` in `src-tauri`).
  - Matchmaker: **81 passed**, 0 failed (`pytest`). Was 66. Added two-host
    deterministic winner, occupied/stale/pending/wrong-tier exclusion, Pro
    GPU id, concurrent 201+409 on the ranked path. 1 Starlette/httpx
    deprecation warning (unchanged).
  - Web: **26 passed**, 0 failed (`vitest run`); `npm run typecheck` clean.
    Unchanged this phase.
- What’s built / verified / needs hardware / assumptions:
  - **Built:** ranking selector on declared caps + Pro dedicated GPU id.
  - **Verified:** pytest / cargo / vitest counts above. No browser pass
    required (Matchmaker-only ranking; harness UI unchanged).
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO unchanged.
  - **Assumptions:** seeded inventory can still be a single Standard host;
    ranking is a no-op until a second approved live host exists.
- Flagged for the next agent:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** No `gate-*-result.md`.
  - Next software phase is **10** (Host Agent tray, pause, status). Do not
    start 11–16 in that session. Do not start 14–15 without asking the human
    (accounts).
  - Latency/reliability ranking stays a later change **inside**
    `RankedHostSelector`, not a second matcher and not in `sessions.py`.
  - Aikido MCP `namespaceStatus: error`; not a scan pass. Run `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; working tree should
  be clean; still ahead of `origin/main`; did not push.
