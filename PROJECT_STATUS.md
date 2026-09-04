# Project Status — last updated: 2026-09-04, after phase 10 (Host Agent tray, pause, status)

## Done

- Specs: architecture + cahier in `docs/` (OS-conditional Standard Lab, GPU
  exclusivity, four planes). On `main`.
- Hardware-gate runbooks (gates remain **OPEN**): `docs/runbooks/gate-fc.md`,
  `docs/runbooks/gate-whpx.md`, `docs/runbooks/gate-pro.md`.
- Execution phases 1–10 **software** on local `main` (not pushed):
  - Host Agent: CPU/RAM via sysinfo; NVIDIA GPUs via nvml-wrapper; config v2
    (`sharedGpuId`); CSP, `freezePrototype`, capabilities off `core:default`;
    Rust Matchmaker client (register/PUT + 10s heartbeats); OS-conditional
    provisioner stubs + opt-in `POST /v1/provision`. Immediate pause writes
    `sharingEnabled` and skips heartbeats. Presence is paused / idle /
    in-session (in-session from local occupancy after stub provision). Earnings
    copy is `not metered yet`. Rust-side tray (tooltip + pause/resume; no JS
    tray APIs). No hypervisor invoked. **89/89** `cargo test` (2026-09-04,
    phase 10).
  - Matchmaker: Argon2id + JWT-from-env; signup/login/me; host inventory
    pending → manual approve; heartbeats; occupancy partial unique indexes
    `sessions_one_active_per_host` / `sessions_one_active_per_user`;
    `HostSelector.select(db, *, lab_tier) -> Host | None` via
    `get_host_selector()` → `RankedHostSelector` (declared CPU then RAM, then
    host id; Pro requires a dedicated GPU id). `HostProvisioner` after
    occupancy commit. **81/81** pytest (2026-09-04, phase 10 re-run). Session
    JSON still has no `intent` field. No latency/reliability columns. No
    Matchmaker pause field.
  - Web harness: signup/login, FR2 intent prompt → `labTier`, request/end
    session, camelCase types aligned with Matchmaker; `isLiveKasmUrl` rejects
    RFC 2606 `.invalid`. **26/26** vitest; `npm run typecheck` clean.

## In Progress

- Cahier Phase 1 overall (MVP Standard Lab). Control-plane software loop is in
  place; a usable desktop still needs hardware gates. Next **software** session
  is execution **phase 11** (session history + billing/credits). Do not start it
  unless the human has chosen a payments approach.

## Up Next

1. Phase 11 — Session history + billing/credits (FR7, FR16). See `PHASE_PLAN.md`.
   Plug-in comments in `sessions.py` already mark the spot. Do not invent a
   ledger if payments are still undecided.
2. Human: GATE-FC / GATE-WHPX / GATE-PRO runbooks when a Linux/Windows host PC
   is available. Agents must not close these.

## Known Risks / Open Items

- **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** Runbooks exist; there is no
  human result file, guest image, hypervisor script, or live
  `SEED_HOST_KASM_URL` (seed example is still `https://example.invalid/kasm`).
  This Mac cannot host. Closing a gate requires the runbook result file **and**
  an explicit human confirmation.
- Local `main` is ahead of `origin/main` (planning + phases 1–10). Nothing was
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
- No Tailscale/Headscale, Storj, or billing. Guest egress via platform exit
  nodes is still on paper (legal exposure at public launch — architecture §9
  item 5). Packaged tray was compiled this phase but not opened in `tauri dev`.
- Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
  `/aikido:setup` if a scan is required.
- Consumer GPU EULAs and ToS/liability still need legal review (architecture
  §9 items 10–11).

## Notes for next agent/session

- You are starting **phase 11** (session history + billing/credits) only if the
  human has chosen a payments approach. Read `PHASE_PLAN.md`, this file, and
  `docs/phase-logs/` (including `phase-10-tray-pause-status.md`).
- Session router uses `HostSelector` + `HostProvisioner` via Depends. Do not
  import `RankedHostSelector` in `sessions.py`. Do not put
  Firecracker/QEMU/WHPX names there. Occupancy stays on session indexes.
- Stale threshold is **45 seconds**. Agent ping interval is **10 seconds**.
  Never-seen hosts are not selectable. Pause is local `sharingEnabled=false`.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.”
