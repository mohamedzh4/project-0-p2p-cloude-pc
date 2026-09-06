# Handoff — phase 3 Host Agent Matchmaker client (2026-09-04)

- Phase goal: Register the Host Agent's clamped v2 caps with the Matchmaker
  from Rust (phase 2 API). Webview still does not `fetch` the Matchmaker.
- Verified git state at start: `cab2e6a` on `main`, working tree clean, branch
  ahead of `origin/main` by 3. Did not push.
- Files created/changed:
  - `host-agent/src-tauri/src/matchmaker.rs` (inventory client: clamp, POST/PUT,
    fail-fast env, token-redacted Debug)
  - `host-agent/src-tauri/src/identity.rs` (`host-identity.json` for host id)
  - `host-agent/src-tauri/src/commands.rs`, `lib.rs`, `Cargo.toml`, `Cargo.lock`
  - `host-agent/src/api.ts`, `App.tsx`, `types.ts`, `styles.css`
  - `host-agent/.env.example`, `.gitignore`, `README.md`, `tauri.conf.json`
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/phase-logs/phase-3-host-agent-client.md` (this file)
- Key decisions made and why:
  - Matchmaker HTTP stays in Rust (`ureq`, redirects disabled so the bootstrap
    bearer is not replayed). The webview only invokes Tauri commands. CSP is
    unchanged (no remote `connect-src`).
  - Caps are re-clamped in Rust before serialize, even if the UI already
    limited inputs. `register_declared_caps` reads the on-disk config, not the
    webview payload.
  - Host id is persisted beside sharing config (`host-identity.json`), not in
    sharing-config v2, so PUT `/v1/hosts/{id}` works without a schema bump.
    First save POSTs; later saves PUT.
  - `kasmUrl` is omitted on the wire so a PUT cannot wipe a URL stored by
    another process. `labTier` is sent as `standard`.
  - Fail-fast if `MATCHMAKER_URL` or `HOST_BOOTSTRAP_TOKEN` is missing/blank.
    Optional `HOST_DISPLAY_NAME` (else hostname). The agent never reads
    `HOST_APPROVE_TOKEN`. New hosts stay pending.
  - Local save still succeeds if registration fails; the UI reports
    "Saved locally, but Matchmaker registration failed: …".
  - GPU exclusivity copy remains on the GPU picker and is repeated on the
    registration status when `shareGpu` is true.
  - UI stayed in this phase (save panel + status). No tray. No 3b split.
  - Did not add heartbeats, provision, Tailscale, or auto-approve.
  - Matchmaker Python was not changed (no contract change).
- Test results:
  - Host Agent: **46 passed**, 0 failed (`cargo test`). Was 30; added identity
    round-trip, payload clamp, GPU id in JSON when `shareGpu`, fail-fast env,
    POST vs PUT, token redaction. `cargo clippy -- -D warnings` clean.
    `npm run typecheck` clean.
  - Matchmaker: **37 passed**, 0 failed (`pytest`). Unchanged.
  - Browser preview (`npm run dev` on :1420): save with GPU dedicated shows
    pending host id and exclusivity copy. Preview simulates registration; the
    packaged agent is the real client. Console 404 on `/favicon.ico` is
    pre-existing.
- New env vars on the **host machine** (copy `host-agent/.env.example`):
  - `MATCHMAKER_URL` (required) — origin, e.g. `http://127.0.0.1:8000`
  - `HOST_BOOTSTRAP_TOKEN` (required) — same value as Matchmaker
  - `HOST_DISPLAY_NAME` (optional) — defaults to hostname
  - Do **not** put `HOST_APPROVE_TOKEN` in the Host Agent env.
- Flagged for the orchestrator:
  - Hardware gates OPEN. Runbooks untouched; no `gate-*-result.md`; seed URL
    still `https://example.invalid/kasm`.
  - No contract changes. Phase 2 host JSON/API unchanged.
  - Shared bootstrap token can still update any host id that knows it.
  - Local `sharingEnabled` is not sent to the Matchmaker. Pause vs live
    inventory is phase 4 (heartbeats).
  - `sharedLogicalCores` / `sharedMemoryBytes` of 0 fail registration (Matchmaker
    `ge=1`) before HTTP; the local sliders still allow 0.
  - Aikido MCP `namespaceStatus: error` / no `aikido_full_scan` tool; not a
    scan pass.
  - `origin/main` still `5425ccf`. Local `main` is planning + phases 1–3.
    Nothing pushed.
  - Next session must start at **phase 4** (heartbeats) and stop there.
