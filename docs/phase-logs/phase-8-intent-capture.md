# Handoff — phase 8 intent capture (2026-09-04)

- Phase goal: FR2 — harness asks “What do you want to do in your lab today?”
  and maps the answer to `labTier` (`standard` / `pro`) on `POST /v1/sessions`.
- Verified git state at start: `874bae0` on `main`, working tree clean, branch
  ahead of `origin/main` by 8. Did not push.
- Files created/changed:
  - `web/src/intent.ts` + `intent.test.ts` (presets, keyword map, GPU copy)
  - `web/src/api.ts` (`requestSession(token, labTier)` — no silent Standard
    default in the client)
  - `web/src/api.test.ts`, `sessionRequest.ts` + `.test.ts` (Pro 409 →
    `unavailable` notice)
  - `web/src/components/SessionPanel.tsx`, `App.tsx`, `types.ts`, `styles.css`
  - `web/README.md`, `host-agent/README.md` (next phase is 9)
  - `matchmaker/tests/test_sessions.py` (Pro 409 docstring + no `intent` field)
  - `PHASE_PLAN.md` (FR2 implemented; suite counts), `PROJECT_STATUS.md`,
    `PROJECT_CONTEXT.md`, `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-8-intent-capture.md` (this file)
- Key decisions made and why:
  - Mapping lives in `web/` only. Matchmaker still accepts `{ labTier }`.
    Intent text is not sent and is not on the session JSON (no contract change).
  - Keyword map follows architecture §3.1 vs §3.2: office/browse/code →
    `standard`; GPU/game/3D/render/training/video-edit → `pro`. Empty input
    does not book. Presets cover both buttons and free text.
  - Pro exclusivity is shown as soon as the mapped tier is Pro (before book)
    and again if a Pro session is held. Not presented as a concurrent slice.
  - Pro 409 when no approved Pro host exists is expected. Harness copy uses
    notice kind `unavailable` for that 409; Standard 409 stays `occupied`.
    Same HTTP `Host unavailable` detail.
  - Did not change `HostSelector.select(db, *, lab_tier) -> Host | None`.
    Did not import `SingleApprovedHostSelector` in the session router.
    Occupancy stays partial unique indexes. Did not iframe Kasm. `.invalid`
    URLs still fail `isLiveKasmUrl`. Did not mark GATE-* closed. Did not
    start Tailscale/Storj.
- Test results:
  - Host Agent: **77 passed**, 0 failed (`cargo test` in `src-tauri`).
  - Matchmaker: **66 passed**, 0 failed (`pytest`). 1 Starlette/httpx
    deprecation warning (unchanged).
  - Web: **26 passed**, 0 failed (`vitest run`); `npm run typecheck` clean.
    Added intent mapping + Pro POST/409 cases; kept `.invalid` live-URL tests.
- What’s built / verified / needs hardware / assumptions:
  - **Built:** FR2 prompt + `mapIntentToLabTier` + Pro GPU exclusivity copy in
    the harness; booking still `POST /v1/sessions` `{ labTier }`.
  - **Verified:** unit/integration suites above. Auth screen in the browser
    (`localhost:5173`) shows the updated lede; no console errors. Signup to
    exercise the intent form was blocked (credential prompt / automation
    policy); mapping and Pro 409 are covered by vitest + pytest.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO unchanged.
  - **Assumptions:** seeded inventory remains Standard-only, so a mapped Pro
    book 409s until a Pro host exists. That is OK for this slice.
- Flagged for the next agent:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** No `gate-*-result.md`.
  - Next software phase is **9** (ranking `HostSelector` swap). Do not start
    10–16 in that session. Do not start 14–15 without asking the human
    (accounts).
  - Session JSON camelCase unchanged. No `intent` field.
  - Aikido MCP `namespaceStatus: error`; not a scan pass. Run `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; working tree should
  be clean; still ahead of `origin/main`; did not push.
