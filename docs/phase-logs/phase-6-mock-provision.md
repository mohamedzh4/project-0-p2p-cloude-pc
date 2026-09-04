# Handoff — phase 6 mock provision (2026-09-04)

- Phase goal: After `HostSelector.select`, Matchmaker calls a Host Agent
  provision **port** (interface, not a hypervisor) and returns the stub
  connection URL as `connection.kasmUrl`.
- Verified git state at start: `5d7ec29` on `main`, working tree clean, branch
  ahead of `origin/main` by 6. Did not push.
- Files created/changed:
  - `matchmaker/src/matchmaker/provision.py` (new: `HostProvisioner` Protocol,
    `HttpHostProvisioner`, `ProvisionResult`)
  - `matchmaker/src/matchmaker/deps.py` (`get_host_provisioner`)
  - `matchmaker/src/matchmaker/routers/sessions.py` (call port after occupancy
    commit; fallback to host-row URL)
  - `matchmaker/src/matchmaker/models.py`, `schemas.py`, `inventory.py`,
    `routers/hosts.py`, `seed.py`, `config.py`
  - `matchmaker/alembic/versions/0003_host_agent_url.py` (additive `agent_url`)
  - `matchmaker/tests/test_provision.py`, `test_sessions.py`, `conftest.py`,
    `helpers.py`
  - `matchmaker/pyproject.toml` (`httpx` runtime), `.env.example`, `README.md`
  - `host-agent/src-tauri/src/provision_http.rs` (new: `POST /v1/provision`)
  - `host-agent/src-tauri/src/lib.rs`, `matchmaker.rs`, `provision.rs`,
    `Cargo.toml`, `Cargo.lock`
  - `host-agent/.env.example`, `README.md`
  - `PHASE_PLAN.md` (stable contract)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/phase-logs/phase-6-mock-provision.md` (this file)
- Key decisions made and why:
  - Provision is a Python `Protocol` injected via `Depends(get_host_provisioner)`.
    Tests swap `FakeHostProvisioner`. Production is `HttpHostProvisioner`.
    `sessions.py` does not import `SingleApprovedHostSelector` and does not name
    Firecracker/QEMU/WHPX.
  - Occupancy commit happens **before** the provision call so concurrent
    double-book stays 201+409 on the partial unique indexes (no app-level lock).
    The 409 loser does not need a ready agent.
  - How Matchmaker finds the agent: `hosts.agent_url` (inventory `agentUrl`),
    else optional env `HOST_AGENT_PROVISION_URL`. If neither is a usable http(s)
    origin, or the HTTP call fails / returns `hypervisorInvoked: true`, keep the
    seeded/placeholder `kasmUrl` on the host row.
  - Additive schema `hosts.agent_url` (nullable). Host Agent advertises it when
    `HOST_AGENT_URL` or `HOST_AGENT_LISTEN_ADDR` is set; omit on PUT to avoid
    wiping. Secrets stay env-only; listen without `HOST_BOOTSTRAP_TOKEN` fails
    fast. `HOST_AGENT_PROVISION_URL` is optional (invalid non-empty fails fast).
  - Session JSON **unchanged**. A successful stub URL is written onto
    `hosts.kasm_url` so existing-session 200 reuses `_session_response` without
    a new field. That is a stored-value side effect, not a response-shape change.
  - Host Agent `POST /v1/provision` calls `plan_provision` with `HostOs::current()`.
    Stub `connectionUrl` is `https://provision.invalid/{backend}`.
    `hypervisorInvoked` is always false. On this Mac the live listener would
    return 400 (`UnsupportedHostOs`); pytest uses a fake, Rust tests pass Linux
    / Windows into `respond`.
  - Webview still does not `fetch` Matchmaker or provision (CSP unchanged).
  - Did not invoke Firecracker/QEMU/Kasm/Sunshine/VFIO. Did not start phase 7.
    Did not add Tailscale/Storj. Did not close GATE-*.
- Test results:
  - Matchmaker: **66 passed**, 0 failed (`pytest`). Was 52. Added book → fake
    agent ready → `kasmUrl` from provision (not the host row); existing 200 does
    not provision again; double-book 409 with a successful stub; concurrent
    201+409 with the fake; HTTP client against a local stub server; fallback /
    401 / `hypervisorInvoked: true` → None; `agentUrl` round-trip / PUT no-wipe
    / 422; router source has no backend names or concrete selector. Concurrent
    booking without a fake still **201+409**. Original 52 still pass (no agent
    URL → host-row fallback).
  - Host Agent: **77 passed**, 0 failed (`cargo test`). Was 66. Added provision
    HTTP: Linux/Windows stub URLs, macOS 400, 401, 404, listen-config skip /
    fail-fast / redacted Debug, bind round-trip `hypervisorInvoked` false,
    inventory `agentUrl` when listen env is set. `cargo clippy -- -D warnings`
    clean. `npm run typecheck` clean.
  - Web suite not re-run (unchanged this session).
- Contract changes other phases depend on:
  - Host JSON: optional `agentUrl` on register/update/response.
  - Matchmaker env: optional `HOST_AGENT_PROVISION_URL`. Seed:
    optional `SEED_HOST_AGENT_URL`.
  - Host Agent env: optional `HOST_AGENT_LISTEN_ADDR`, `HOST_AGENT_URL`.
  - Host Agent `POST /v1/provision` (bootstrap Bearer) →
    `{ connectionUrl, hypervisorInvoked, gpuExclusive }` (camelCase).
  - Session response shape **not** changed.
- Flagged for the orchestrator:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** Runbooks untouched; no
    `gate-*-result.md`; seed URL still `https://example.invalid/kasm`.
  - On macOS, a live provision listener cannot return a stub URL (OS unsupported).
    Local harness booking still uses the seeded placeholder unless a fake /
    Linux-Windows agent is in the path.
  - Successful provision overwrites `hosts.kasm_url` with the stub URL.
  - Shared bootstrap token can still update, ping, or provision any host that
    knows it.
  - Aikido MCP `namespaceStatus: error` / no usable `aikido_full_scan`; not a
    scan pass.
  - `origin/main` still behind local `main`. Nothing pushed.
  - Next session must start at **phase 7** (orchestrator MVP pass) and stop
    there.
- Verified git state at end: this commit on local `main`; working tree should
  be clean; still ahead of `origin/main`; did not push.
