# Handoff — phase 14 self-hosted Headscale mesh (2026-09-05)

- Phase goal: FR9 network slice — self-hosted Headscale (no Tailscale SaaS);
  Matchmaker mints a Host Agent preauth key; Host Agent joins from Rust.
- Verified git state at start: `a1d5ab0` on `main`, working tree clean, branch
  ahead of `origin/main` by 14. Did not push. GATE-FC / GATE-WHPX / GATE-PRO
  remain OPEN. `PHASE_PLAN.md` phase 14 is Tailscale/Headscale mesh. Human
  chose option 1 (no Tailscale cloud account).
- Files created/changed:
  - `headscale/docker-compose.yml`, `headscale/config.yaml`,
    `headscale/.env.example`
  - `matchmaker/src/matchmaker/headscale.py`
  - `matchmaker/src/matchmaker/config.py`, `deps.py`, `schemas.py`,
    `routers/hosts.py`, `routers/sessions.py`
  - `matchmaker/tests/test_headscale.py`, `tests/test_config.py`,
    `tests/conftest.py`
  - `matchmaker/.env.example`, `matchmaker/README.md`
  - `host-agent/src-tauri/src/mesh.rs`
  - `host-agent/src-tauri/src/lib.rs`, `commands.rs`, `matchmaker.rs`
  - `host-agent/.env.example`, `host-agent/README.md`
  - `docs/runbooks/headscale.md`
  - `docs/phase-logs/phase-14-headscale-mesh.md` (this file)
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `.gitignore`
- Key decisions made and why:
  - **Self-hosted Headscale 0.26.1**, not Tailscale cloud. Compose requires
    `HEADSCALE_SERVER_URL` (fail if missing), same interpolation pattern as
    matchmaker/postgres. `server_url` in `config.yaml` must match that env
    (Headscale reads the YAML, not compose env).
  - **Matchmaker mints** `POST /v1/hosts/{id}/mesh/preauth` (host bootstrap
    Bearer) via `HEADSCALE_URL` + `HEADSCALE_API_KEY` + `HEADSCALE_USER`.
    Single-use, non-ephemeral host key, 1h TTL. API key never returned.
    Redirects refused. `HEADSCALE_LOGIN_SERVER` is what agents pass to
    `tailscale up`.
  - **Host Agent join is Rust**, not webview fetch. `HEADSCALE_LOGIN_SERVER`
    from env is the login-server (not blindly the Matchmaker response).
    Missing Matchmaker/mesh env fails fast. No `HEADSCALE_API_KEY` on the
    agent. Join args never include `--advertise-exit-node`.
  - **Join is opt-in** when `HEADSCALE_LOGIN_SERVER` is set (after register).
    UI-only machines omit it. If set, missing Tailscale binary fails the
    register command.
  - **ACLs are a documented stub.** `policy.path` is empty — that is not
    default-deny (architecture §9 item 4). Per-session tagged identities are
    not minted on book. Flagged **14c**.
  - **Exit-node egress is not done** (architecture §9 item 5). Runbook §6
    plus TODOs in `headscale.py` / `mesh.rs`. Volunteer hosts must not become
    exits. Flagged **14c**.
  - **No live Headscale in unit tests.** httpx `MockTransport` + FastAPI fake.
    Pytest overwrites `HEADSCALE_*` to `.invalid` so a local `.env` cannot
    point tests at a live instance.
  - **Did not** change `HostSelector.select`, occupancy indexes, image
    signing, GPU exclusivity, OS split, CSP, `freezePrototype`, Argon2id, or
    JWT. Did not start Storj. Did not close GATE-*.
- How to run Headscale: numbered steps in `docs/runbooks/headscale.md`
  (compose up → users create → apikeys create → Matchmaker `.env` → mint →
  agent join).
- Key env names:
  - Headscale compose: `HEADSCALE_SERVER_URL` (required), `HEADSCALE_PORT`
  - Matchmaker (startup): `HEADSCALE_URL`, `HEADSCALE_LOGIN_SERVER`,
    `HEADSCALE_API_KEY`, `HEADSCALE_USER` (numeric id on 0.26.1)
  - Host Agent (to join): `HEADSCALE_LOGIN_SERVER`, optional `TAILSCALE_BIN`
- Test results:
  - Matchmaker: **123 passed**, 0 failed. Was 113. Added Headscale env
    fail-fast + repr redaction, mocked mint HTTP, mint route auth/404/502,
    source guard.
  - Host Agent: **116 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 109. Added missing-env fail-fast, mint POST + `tailscale up` from
    env, no exit-node flag, no secrets in production source. `cargo clippy
    -- -D warnings` clean on the lib.
  - Web: **27 passed**, 0 failed. `npm run typecheck` clean.
- What’s built / verified / needs hardware / still paper:
  - **Built:** compose + mint API + Rust join contract.
  - **Verified:** pytest/cargo/vitest counts above. No live Headscale smoke.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO unchanged.
  - **Still paper (14c):** default-deny / per-session ACLs; platform exit-node
    guest egress. Do not treat mesh join as solving volunteer ISP exposure.
- Flagged for the next agent:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** No `gate-*-result.md`.
  - Next software phase is **15** (Storj). **Needs a human Storj account —
    do not implement unattended.**
  - Restart Matchmaker with `HEADSCALE_*` set. Do not commit `.env`.
  - Aikido MCP `namespaceStatus: error`; not a scan pass. Run `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; working tree should
  be clean; still ahead of `origin/main`; did not push.
