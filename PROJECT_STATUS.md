# Project Status — last updated: 2026-09-04, after phase 9 (ranking HostSelector swap)

## Done

- Specs: architecture + cahier in `docs/` (OS-conditional Standard Lab, GPU
  exclusivity, four planes). On `main`.
- Hardware-gate runbooks (gates remain **OPEN**): `docs/runbooks/gate-fc.md`,
  `docs/runbooks/gate-whpx.md`, `docs/runbooks/gate-pro.md`.
- Execution phases 1–9 **software** on local `main` (not pushed):
  - Host Agent: CPU/RAM via sysinfo; NVIDIA GPUs via nvml-wrapper; config v2
    (`sharedGpuId`); CSP, `freezePrototype`, capabilities off `core:default`;
    Rust Matchmaker client (register/PUT + 10s heartbeats); OS-conditional
    provisioner stubs + opt-in `POST /v1/provision`. No hypervisor invoked.
    **77/77** `cargo test` (2026-09-04, phase 9 re-run).
  - Matchmaker: Argon2id + JWT-from-env; signup/login/me; host inventory
    pending → manual approve; heartbeats; occupancy partial unique indexes
    `sessions_one_active_per_host` / `sessions_one_active_per_user`;
    `HostSelector.select(db, *, lab_tier) -> Host | None` via
    `get_host_selector()` → `RankedHostSelector` (declared CPU then RAM, then
    host id; Pro requires a dedicated GPU id). `HostProvisioner` after
    occupancy commit. **81/81** pytest (2026-09-04, phase 9). Session JSON
    still has no `intent` field. No latency/reliability columns.
  - Web harness: signup/login, FR2 intent prompt → `labTier`, request/end
    session, camelCase types aligned with Matchmaker; `isLiveKasmUrl` rejects
    RFC 2606 `.invalid`. **26/26** vitest; `npm run typecheck` clean.

## In Progress

- Cahier Phase 1 overall (MVP Standard Lab). Control-plane software loop is in
  place; a usable desktop still needs hardware gates. Next **software** session
  is execution **phase 10** (Host Agent tray, pause, status). Do not start it
  in the session that is wrapping phase 9.

## Up Next

1. Phase 10 — Host Agent tray, pause, status (FR10–FR11). See `PHASE_PLAN.md`.
   Keep CSP / `freezePrototype` / narrowed capabilities. Do not `fetch` the
   Matchmaker from the webview.
2. Human: GATE-FC / GATE-WHPX / GATE-PRO runbooks when a Linux/Windows host PC
   is available. Agents must not close these.

## Known Risks / Open Items

- **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** Runbooks exist; there is no
  human result file, guest image, hypervisor script, or live
  `SEED_HOST_KASM_URL` (seed example is still `https://example.invalid/kasm`).
  This Mac cannot host. Closing a gate requires the runbook result file **and**
  an explicit human confirmation.
- Local `main` is ahead of `origin/main` (planning + phases 1–9). Nothing was
  pushed this session.
- Host register/heartbeat/provision auth uses a shared `HOST_BOOTSTRAP_TOKEN`.
  Per-host tokens are not in this MVP.
- Existing local `matchmaker/.env` must have `HOST_BOOTSTRAP_TOKEN` and
  `HOST_APPROVE_TOKEN` (different values) or uvicorn fails at startup. Pytest
  setdefaults only when they are missing. Do not commit `.env`.
- A long-running Matchmaker on `:8000` (PID from before host-inventory routes)
  still answers signup/sessions but **404s** `POST /v1/hosts`. Do not treat it
  as the phase-6/7 API. Restart uvicorn after adding the host tokens.
- Heartbeats mark inventory stale for **new** bookings only. An active session
  is not interrupted when pings stop.
- Provisioner is stubs only. On macOS, a real Host Agent `POST /v1/provision`
  is 400 (`UnsupportedHostOs`); Matchmaker then keeps the seeded `kasmUrl`.
- Intent text stays in the harness. It is not persisted and is not on the
  session JSON. Keyword mapping is a smallest-slice heuristic (office/code →
  Standard; GPU/game/3D/training → Pro).
- Ranking uses declared caps on the host row. There is no measured latency or
  reliability score. A concurrent race on the same top-ranked host is still
  201+409 (no re-select of the runner-up on IntegrityError; that is phase 12).
- No Tailscale/Headscale, Storj, billing, or tray UI. Guest egress via
  platform exit nodes is still on paper (legal exposure at public launch —
  architecture §9 item 5).
- Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
  `/aikido:setup` if a scan is required.
- Consumer GPU EULAs and ToS/liability still need legal review (architecture
  §9 items 10–11).

## Notes for next agent/session

- You are starting **phase 10** (Host Agent tray, pause, status). Read
  `PHASE_PLAN.md`, this file, and `docs/phase-logs/` (including
  `phase-9-ranked-host-selector.md`).
- Session router uses `HostSelector` + `HostProvisioner` via Depends. Do not
  import `RankedHostSelector` in `sessions.py`. Do not put
  Firecracker/QEMU/WHPX names there. Occupancy stays on session indexes.
- Stale threshold is **45 seconds**. Agent ping interval is **10 seconds**.
  Never-seen hosts are not selectable.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.”
