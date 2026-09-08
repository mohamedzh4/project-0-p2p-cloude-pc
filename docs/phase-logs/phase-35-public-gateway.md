# Handoff — phase 35 public gateway / internet ingress (2026-09-08)

- Phase goal: Zero-install Standard Lab (browser only) — public HTTPS
  gateway reverse-proxies a scoped token to the session’s mesh Kasm/Sunshine
  URL so the client does not join the tailnet.
- Verified git state at start: local `main` after phase 33; did not push.
  Did not invent a domain. Did not switch `RENTERD_NETWORK` off zen. Did not
  claim a cross-city or metal confirmation.
- Files created/changed:
  - `gateway/` (proxy service, Caddy compose, tests)
  - `matchmaker/src/matchmaker/ingress.py`, `routers/gateway.py`,
    `config.py`, `models.py`, `schemas.py`, `deps.py`, `main.py`,
    `routers/sessions.py`, `silence.py`, `headscale.py`
  - `matchmaker/alembic/versions/0007_session_ingress_tokens.py`
  - `matchmaker/tests/test_ingress.py`, `test_headscale.py`, `test_config.py`,
    `conftest.py`
  - `headscale/acl.hujson` (`tag:gateway` ↔ `tag:guest` only)
  - `docs/runbooks/gateway.md`, `docs/runbooks/sia-mainnet.md`
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`,
    `docs/distributed-compute-lab-architecture.md`,
    `docs/runbooks/headscale.md`, `docs/runbooks/renterd.md`
  - `docs/phase-logs/phase-35-public-gateway.md` (this file)
- Key decisions made and why:
  - **Gateway is operator infra**, not a volunteer host. Joins Headscale as
    `tag:gateway` (CLI mint, same pattern as `tag:exit`). Public TLS is
    Caddy/ACME; the process binds loopback only.
  - **Scoped session tokens** (architecture §9 item 9): unguessable
    `token_urlsafe(32)`, SHA-256 lookup, one token per active session,
    expires on end/interrupt/TTL. Same token is reused for that session’s
    many HTTP/WebSocket requests so Kasm can load. Wrong/expired → 404, no
    default backend. Rate-limit failed lookups per IP.
  - **Matchmaker still stores the mesh locator** on `hosts.kasm_url`.
    Session JSON locators become `https://<GATEWAY_PUBLIC_ORIGIN>/s/<token>/`
    only when that origin is set. Unset keeps current mesh URLs (pytest).
    Moonlight `host` stays mesh (Pro native client is not browser-only).
  - **ACL:** `tag:gateway` ↔ `tag:guest` only. **Headscale 0.26 still
    cannot mint pairwise `tag:session-<uuid>` owners** — every gateway node
    may still reach every guest, same leftover as `tag:user`. Token mapping
    is the session scope. No grant to `tag:host` / `tag:user` /
    `tag:matchmaker` / `tag:exit` / `autogroup:internet`.
  - **Did not** change Standard vs Pro provisioner selection, HostSelector,
    occupancy indexes, or hypervisor invoke. Phase 34 TAP is leftover.
  - **Storage:** renterd / Sia / per-user bucket / guest rclone / no keys on
    the wire — unchanged. Mainnet is `docs/runbooks/sia-mainnet.md` only.
- Test results:
  - Matchmaker: **178 passed**, 0 failed. Was 166. Ingress mint/lookup,
    wrap-when-configured, resolve 401/404 fail-closed, end invalidates
    token, ACL gateway grants/denials, gateway env fail-fast.
  - Gateway: **10 passed**, 0 failed (token mapping, ACL file scope, HTTP
    proxy to resolved target only, rate-limit still 404).
  - Host Agent: **328/328** `cargo test --lib` (unchanged).
  - Web: **53/53** vitest (was 52); `npm run typecheck` passed.
  - Guest-images: **12/12** unittest.
- What’s built / verified / needs hardware / still paper:
  - **Built:** gateway service + Matchmaker resolve + public URL on session
    JSON + `tag:gateway` ACL. Implemented in code, not yet run across real
    networks.
  - **Verified:** unit/integration tests above. No live Headscale, no live
    ACME, no public domain.
  - **Needs from the human:** domain they control DNS for; VPS choice
    (same as exit node vs separate); Sia mainnet go-ahead + funded wallet
    before any `RENTERD_NETWORK` change.
  - **Still paper:** TAP (phase 34); pairwise ACLs; live guest net;
    cross-city confirmation.
- Flagged for the next agent:
  - Do not invent a domain. Do not mark “confirmed over the internet”.
  - Do not switch renterd off zen unprompted.
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass.
    Run `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; still ahead of
  `origin/main`; did not push.
