# Handoff — phase 16 Pro/Gamer software (2026-09-05)

- Phase goal: Pro session JSON and provision stubs speak Sunshine/Moonlight
  (exclusive GPU) instead of renaming Kasm. GATE-PRO stays OPEN.
- Verified git state at start: `9f69179` on `main` (locators-only renterd).
  Phase 15 race: `0a59f68` had S3 keys on session JSON; `9f69179` kept keys in
  env. Kept `9f69179`. That commit also leaked incomplete Phase 16 schema
  (`ConnectionInfo.streamer`, `session_connection` import, FakeHostProvisioner
  sunshine kwargs) without `provision.py`. Parallel Phase 16 left dirty
  web/tests plus unused `0007_host_latency_ms.py` — discarded the latency
  column (do not fake FR13). Did not reset --hard. Did not push.
  GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.
- Files created/changed:
  - `matchmaker/src/matchmaker/provision.py` (`session_connection`,
    `STREAMER_*`, Pro locator on `ProvisionResult`)
  - `matchmaker/tests/test_provision.py`, `test_sessions.py`, `test_matching.py`
  - `host-agent/src-tauri/src/provision_http.rs` (Pro `streamer` /
    `sunshineUrl` / `moonlight`)
  - `web/src/connection.ts`, `connection.test.ts`, `types.ts`,
    `components/SessionPanel.tsx`
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `matchmaker/README.md`, `host-agent/README.md`, `web/README.md`
  - `docs/phase-logs/phase-16-pro-gamer.md` (this file)
- Key decisions made and why:
  - **Locators-only storage won** (`9f69179`). No S3 keys on `/v1/storage` or
    session JSON. renterd, not Storj.
  - **Standard = Kasm, Pro = Sunshine/Moonlight** on `connection`. Not Kasm
    renamed. Locator still stored in `hosts.kasm_url` (no extra column).
  - **HostSelector.select unchanged.** Occupancy indexes unchanged. Pro still
    requires `shareGpu` + non-empty `sharedGpuId`. Blank GPU id is not
    selected.
  - **No `latency_ms` column.** Ranking stays declared CPU then RAM. Put
    measured latency in `RankedHostSelector` later — do not fake a score.
  - **VFIO / Hyper-V remain stubs.** HTTP Pro response adds streamer fields;
    `hypervisorInvoked` stays false. GATE-PRO is still a human hardware gate.
- Test results:
  - Matchmaker: **150 passed**, 0 failed. Was 145. Added Sunshine HTTP parse,
    Standard vs Pro `session_connection`, blank GPU skip, Pro session JSON
    without storage keys, FakeHostProvisioner Sunshine locator.
  - Host Agent: **123 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 121. Added Linux Pro Sunshine JSON and Windows Hyper-V + Sunshine.
  - Web: **31 passed**, 0 failed. Was 27. `npm run typecheck` passed.
- What’s built / verified / needs hardware / still paper:
  - **Built:** Pro vs Standard connection contract; exclusive GPU matching;
    Sunshine locators on stub provision.
  - **Verified:** pytest/cargo/vitest counts above. Harness shows Sunshine
    exclusivity copy for Pro intent (live stream still `.invalid`).
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO unchanged.
  - **Still paper:** live VFIO/Hyper-V; Sunshine in a guest; Moonlight RTT;
    FR13 measured latency; guest rclone.
- Flagged for the next agent:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** No `gate-*-result.md`.
  - No numbered software phase after 16. Do not close GATE-*. Do not add
    Storj. Do not fake latency. Do not put keys on storage JSON.
  - Long-running uvicorn on `:8000` from before this commit will not emit
    `streamer` / `sunshineUrl`. Restart Matchmaker after pull.
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; working tree
  should be clean; still ahead of `origin/main`; did not push.
