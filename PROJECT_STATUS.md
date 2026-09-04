# Project Status — last updated: 2026-09-04, after phase 11 (session history + internal credits ledger)

## Done

- Specs: architecture + cahier in `docs/` (OS-conditional Standard Lab, GPU
  exclusivity, four planes). On `main`.
- Hardware-gate runbooks (gates remain **OPEN**): `docs/runbooks/gate-fc.md`,
  `docs/runbooks/gate-whpx.md`, `docs/runbooks/gate-pro.md`.
- Execution phases 1–11 **software** on local `main` (not pushed):
  - Host Agent: CPU/RAM via sysinfo; NVIDIA GPUs via nvml-wrapper; config v2
    (`sharedGpuId`); CSP, `freezePrototype`, capabilities off `core:default`;
    Rust Matchmaker client (register/PUT + 10s heartbeats + earnings GET).
    OS-conditional provisioner stubs + opt-in `POST /v1/provision`. Immediate
    pause writes `sharingEnabled` and skips heartbeats. Presence is paused /
    idle / in-session. Earnings copy is `{n} credits accrued (not paid out)`
    from the Matchmaker ledger (not a payout). Rust-side tray (tooltip +
    pause/resume; no JS tray APIs). No hypervisor invoked. **93/93**
    `cargo test` (2026-09-04, phase 11).
  - Matchmaker: Argon2id + JWT-from-env; signup/login/me; host inventory
    pending → manual approve; heartbeats; occupancy partial unique indexes
    `sessions_one_active_per_host` / `sessions_one_active_per_user`;
    `HostSelector.select(db, *, lab_tier) -> Host | None` via
    `get_host_selector()` → `RankedHostSelector`. `HostProvisioner` after
    occupancy commit. Session end meters `startedAt` → `endedAt` into
    `ledger_entries` (user debit + host credit). `GET /v1/sessions` history;
    `GET /v1/hosts/{id}/earnings` accrued credits. `CREDITS_PER_MINUTE` is
    required env (non-negative int). **94/94** pytest (2026-09-04, phase 11).
    Session JSON still has no `intent` field. No latency/reliability columns.
    No Matchmaker pause field. No payment processor.
  - Web harness: signup/login, FR2 intent prompt → `labTier`, request/end
    session, session history + credits debited, camelCase types aligned with
    Matchmaker; `isLiveKasmUrl` rejects RFC 2606 `.invalid`. **27/27** vitest;
    `npm run typecheck` clean.

## In Progress

- Cahier Phase 1 overall (MVP Standard Lab). Control-plane software loop is in
  place; a usable desktop still needs hardware gates. Next **software** session
  is execution **phase 12** (heartbeat-silence teardown + re-match).

## Up Next

1. Phase 12 — Heartbeat-silence teardown + re-match (FR14 remainder, FR15).
   See `PHASE_PLAN.md`. Occupancy stays on session indexes. Do not put
   hypervisor names in `sessions.py`.
2. Human: GATE-FC / GATE-WHPX / GATE-PRO runbooks when a Linux/Windows host PC
   is available. Agents must not close these.

## Known Risks / Open Items

- **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** Runbooks exist; there is no
  human result file, guest image, hypervisor script, or live
  `SEED_HOST_KASM_URL` (seed example is still `https://example.invalid/kasm`).
  This Mac cannot host. Closing a gate requires the runbook result file **and**
  an explicit human confirmation.
- Local `main` is ahead of `origin/main` (planning + phases 1–11). Nothing was
  pushed this session.
- Host register/heartbeat/provision/earnings auth uses a shared
  `HOST_BOOTSTRAP_TOKEN`. Per-host tokens are not in this MVP.
- Existing local `matchmaker/.env` must have `HOST_BOOTSTRAP_TOKEN` and
  `HOST_APPROVE_TOKEN` (different values) plus `CREDITS_PER_MINUTE` or uvicorn
  fails at startup. Pytest setdefaults only when they are missing. Do not
  commit `.env`.
- A long-running Matchmaker on `:8000` from before this phase will not have
  ledger routes or `CREDITS_PER_MINUTE`. Restart uvicorn and `alembic upgrade
  head` after pulling this commit.
- Heartbeats mark inventory stale for **new** bookings only. An active session
  is not interrupted when pings stop. Pause also does not end an active session.
- Local Host Agent occupancy (in-session) is set on stub provision 200 and is
  **not cleared** until phase 12 teardown. Resume after pause can still show
  in-session if a guest already left.
- Provisioner is stubs only. On macOS, a real Host Agent `POST /v1/provision`
  is 400 (`UnsupportedHostOs`); Matchmaker then keeps the seeded `kasmUrl`.
- Intent text stays in the harness. It is not persisted and is not on the
  session JSON. Keyword mapping is a smallest-slice heuristic (office/code →
  Standard; GPU/game/3D/training → Pro).
- Ranking uses declared caps on the host row. There is no measured latency or
  reliability score. A concurrent race on the same top-ranked host is still
  201+409 (no re-select of the runner-up on IntegrityError; that is phase 12).
- **Real host payouts and user card charges are deferred.** The ledger records
  credits only. Do not add Stripe/PayPal.
- No Tailscale/Headscale or Storj. Guest egress via platform exit nodes is
  still on paper (legal exposure at public launch — architecture §9 item 5).
- Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
  `/aikido:setup` if a scan is required.
- Consumer GPU EULAs and ToS/liability still need legal review (architecture
  §9 items 10–11).

## Notes for next agent/session

- You are starting **phase 12** (heartbeat-silence teardown + re-match). Read
  `PHASE_PLAN.md`, this file, and `docs/phase-logs/` (including
  `phase-11-session-ledger.md`).
- Session router uses `HostSelector` + `HostProvisioner` via Depends. Do not
  import `RankedHostSelector` in `sessions.py`. Do not put
  Firecracker/QEMU/WHPX names there. Occupancy stays on session indexes.
- Metering is `ledger.settle_ended_session` at end. Unique
  `(session_id, party)` prevents double-credit. Rate is `CREDITS_PER_MINUTE`.
- Stale threshold is **45 seconds**. Agent ping interval is **10 seconds**.
  Never-seen hosts are not selectable. Pause is local `sharingEnabled=false`.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.”
