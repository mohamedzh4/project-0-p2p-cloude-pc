# Handoff — phase 32 web full loop (2026-09-07)

- Phase goal: Finish the `web/` harness loop — clickable live Kasm/Sunshine
  locator, automatic FR2/FR3 match, end/history/credits from Matchmaker.
- Recovered a dirty tree from a previous phase-32 agent (PING timeout). No
  phase-32 commit existed. Did not rewrite `1f5890e`…`9a61be0`. Did not push.
  Did not touch `pr/*`.
- Files created/changed:
  - `web/src/connection.ts` + `connection.test.ts` (`hostnameIsPlaceholder`,
    `sessionDesktopLink`, `sessionMoonlightClient`; phase-31 `/` and `:47990/`)
  - `web/src/sessionRequest.ts` + `.test.ts` (`sessionBookBody`, restore from
    history, `endedNotice`, `creditsSummary`)
  - `web/src/api.ts` + `api.test.ts` (end route + `labTier`-only POST body)
  - `web/src/components/SessionPanel.tsx` + `SessionPanel.test.tsx` (clickable
    Open desktop / Open Sunshine web UI; no host picker; no iframe)
  - `web/src/App.tsx`, `types.ts`, `styles.css`, `package.json`, `web/README.md`
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `host-agent/README.md`, `matchmaker/README.md` (fixed a duplicated intro
    paragraph from the recovered draft)
  - `docs/phase-logs/phase-32-web-full-loop.md` (this file)
- Key decisions made and why:
  - Matching stays automatic: POST `{ labTier }` only. No host catalog, no
    picker. Flagged: do not switch to manual browsing unless a later product
    decision says so.
  - Standard live locator → **Open desktop** (`https://{meshHost}/`). Pro live
    locator → **Open Sunshine web UI** (`https://{meshHost}:47990/`) plus
    Moonlight host/app when that host is not `.invalid`. Moonlight is the
    desktop client; the Sunshine URL is the guest web UI, not an iframe.
  - `isLiveConnectionUrl` hides RFC 2606 `.invalid` (seed + provision stubs).
  - Host inventory is Matchmaker register + heartbeat + manual approve. The
    harness does not fake capacity.
  - Login restores an `active` row via `GET /v1/sessions` then the existing
    book route (200). End/history/credits use `POST .../end` and `GET /v1/sessions`.
  - Hardware lines stay **Implemented in code. Not yet run on physical
    hardware.** Did not rewrite Firecracker/QEMU/VFIO/Hyper-V internals.

## Test results

- Web: **52 passed**, 0 failed (`npm test` / vitest). Was 31 (+21).
  `npm run typecheck` clean.
- Matchmaker pytest / Host Agent cargo: not run (untouched this phase).
- Browser (Playwright against already-running `http://localhost:5173`):
  signup → intent form shows auto-match copy and **no host picker**;
  Standard preset maps to CPU/RAM slice; Pro preset shows GPU exclusivity +
  Sunshine/Moonlight (not Kasm). `POST /v1/sessions` returned **409 occupied**
  (seed host already held). `GET /v1/sessions` is **405** on the five-day-old
  uvicorn on `:8000` (no history route). Clickable live locator was **not**
  exercised against that process — covered by vitest (`sessionDesktopLink`,
  SessionPanel render). Restart uvicorn after this commit for a live book.
- Aikido: MCP `namespaceStatus: error`; `mcp_auth` timed out. Not a scan
  pass. Run `/aikido:setup`.

New / moved tests:

- `isLiveConnectionUrl` phase-31 Kasm `/` and Sunshine `:47990/`
- `hostnameIsPlaceholder`, `sessionDesktopLink`, `sessionMoonlightClient`
- `sessionBookBody` (no `hostId`), `activeSessionLabTier`, `endedNotice`,
  `creditsSummary`
- `endSession` posts `/v1/sessions/{id}/end`
- `SessionPanel` clickable Open desktop / Open Sunshine web UI; hides
  `.invalid`; no host picker / iframe (`SessionPanel.test.tsx`)

## Flagged for the next agent

- **Next: Host Agent binary** (`cd host-agent && npm run tauri build`). This
  Mac cannot produce Linux/Windows installers. Phase 33 owns packaging gaps.
- Do not switch matching to a host catalog. Auto-match is the product rule.
- Hardware is still not run on physical hardware. Do not invent a live desktop.
- `SEED_HOST_KASM_URL` remains `https://example.invalid/kasm` for the seed
  command. A book against that seed is 502 unless provision returns a live URL.
- A long-running Matchmaker from 2026-09-02 405s on `GET /v1/sessions` and
  occupied the seed host (409). Restart uvicorn after this commit.
- `origin/main` still behind. Nothing pushed.
