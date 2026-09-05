# Project Status — last updated: 2026-09-05, after phase 15 (renterd, not Storj)

## Done

- Specs: architecture + cahier in `docs/` (OS-conditional Standard Lab, GPU
  exclusivity, four planes). On `main`.
- Hardware-gate runbooks (gates remain **OPEN**): `docs/runbooks/gate-fc.md`,
  `docs/runbooks/gate-whpx.md`, `docs/runbooks/gate-pro.md`.
- Headscale operator runbook: `docs/runbooks/headscale.md` (ACL grants +
  platform exit-node steps).
- renterd operator runbook: `docs/runbooks/renterd.md` (compose, S3 v4
  keypairs, guest rclone). **Implemented storage is renterd, not Storj
  (human decision 2026-09-05).**
- Execution phases 1–15 **software** on local `main` (not pushed):
  - Host Agent: CPU/RAM via sysinfo; NVIDIA GPUs via nvml-wrapper; config v2
    (`sharedGpuId`); CSP, `freezePrototype`, capabilities off `core:default`;
    Rust Matchmaker client (register/PUT + 10s heartbeats + earnings GET).
    OS-conditional provisioner stubs + opt-in `POST /v1/provision`. Immediate
    pause writes `sharingEnabled` and skips heartbeats. Presence is paused /
    idle / in-session. Earnings copy is `{n} credits accrued (not paid out)`
    from the Matchmaker ledger (not a payout). Rust-side tray (tooltip +
    pause/resume; no JS tray APIs). No hypervisor invoked. **Before** a stub
    boot, `image.rs` verifies Ed25519 over the SHA-256 digest against
    `IMAGE_SIGNING_PUBLIC_KEY` and `IMAGE_LOCAL_DIGEST` (required when the
    provision listener is on). After register, if `HEADSCALE_LOGIN_SERVER` is
    set, `mesh.rs` POSTs `/v1/hosts/{id}/mesh/preauth` from Rust and runs
    `tailscale up` with that env URL + the minted key (never
    `--advertise-exit-node`, `--exit-node`, or `--advertise-tags`). Provision
    may include `storageEndpoint` (URL only); the stub never rclone-mounts
    on the host OS. `storage.rs` can plan guest rclone argv from public
    locators and does not read S3 secrets. **121/121** `cargo test` (2026-09-05).
  - Matchmaker: Argon2id + JWT-from-env; signup/login/me; host inventory
    pending → manual approve; heartbeats; occupancy partial unique indexes
    `sessions_one_active_per_host` / `sessions_one_active_per_user`;
    `HostSelector.select(db, *, lab_tier) -> Host | None` via
    `get_host_selector()` → `RankedHostSelector`. `HostProvisioner` after
    occupancy commit, now advertising `imageDigest` + `imageSignature` for the
    booked lab tier plus optional `storageEndpoint`. `GET /v1/images/{labTier}`
    is checksum metadata only. `POST /v1/hosts/{id}/mesh/preauth` mints a
    Headscale preauth key tagged **`tag:host`**. `GET /v1/storage` and session
    JSON `storage` provision a per-user renterd S3 bucket (`dcl-<uuid>`) and
    a guest rclone command (keys for the guest, not host disk). Bucket
    provision runs **before** occupy; renterd failure is 502 without taking
    the host. Session end meters `startedAt` → `endedAt` into `ledger_entries`.
    Silence (~45s) on an occupied host marks the session `interrupted`,
    settles the ledger once, and rematches via one `select` (same renterd
    bucket follows the user; rematch does not call renterd). Required env
    includes `CREDITS_PER_MINUTE`, the four `IMAGE_*` pairs, `HEADSCALE_*`,
    and `RENTERD_S3_ENDPOINT` / `RENTERD_S3_ACCESS_KEY` /
    `RENTERD_S3_SECRET_KEY`. **143/143** pytest (2026-09-05). Session JSON
    still has no `intent` field. No latency/reliability columns. No
    Matchmaker pause field. No payment processor. No image CDN / multi-GB
    download. No live Sia network in unit tests.
  - Web harness: signup/login, FR2 intent prompt → `labTier`, request/end
    session, session history + credits debited, optional storage locator
    (endpoint/bucket only — not secrets). **27/27** vitest;
    `npm run typecheck` not re-run this session beyond vitest.
  - Headscale compose in `headscale/` (image `0.26.1`) with checked-in
    default-deny `acl.hujson`. renterd compose in `renterd/` (image
    `ghcr.io/siafoundation/renterd:2.9.3`, zen testnet, S3 on loopback
    **8081**). No Tailscale cloud account. No Storj.

## In Progress

- Cahier Phase 1 overall (MVP Standard Lab). Control-plane software loop is in
  place; a usable desktop still needs hardware gates. Next **software** session
  is execution **phase 16** (Pro/Gamer automation) — blocked on **GATE-PRO**
  for a real session.

## Up Next

1. Phase 16 — Pro/Gamer automation (cahier 3). Do not close GATE-* from that
   work. Live guest rclone / “saved as of HH:MM” still need GATE-FC /
   GATE-WHPX plus a guest image.
2. Human: GATE-FC / GATE-WHPX / GATE-PRO runbooks when a Linux/Windows host PC
   is available. Agents must not close these.

## Known Risks / Open Items

- **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** Runbooks exist; there is no
  human result file, guest image, hypervisor script, or live
  `SEED_HOST_KASM_URL` (seed example is still `https://example.invalid/kasm`).
  This Mac cannot host. Closing a gate requires the runbook result file **and**
  an explicit human confirmation.
- Local `main` is ahead of `origin/main` (planning + phases 1–15). Nothing was
  pushed this session.
- Host register/heartbeat/provision/earnings/mesh-preauth auth uses a shared
  `HOST_BOOTSTRAP_TOKEN`. Per-host tokens are not in this MVP.
- Existing local `matchmaker/.env` must have `HOST_BOOTSTRAP_TOKEN` and
  `HOST_APPROVE_TOKEN` (different values), `CREDITS_PER_MINUTE`, the four
  `IMAGE_*` digest/signature pairs, `HEADSCALE_*`, and `RENTERD_S3_*` or
  uvicorn fails at startup. Pytest overwrites Headscale and renterd env to
  `.invalid` so a local `.env` cannot point tests at a live instance. Do not
  commit `.env`, renterd seeds, or `renterd/data/`. Copy fixture `IMAGE_*`
  values from `.env.example` for local stub verify; they are not a production
  key. Optional `STALE_SESSION_SWEEP_INTERVAL_SECONDS` defaults to 15.
- Enabling `HOST_AGENT_LISTEN_ADDR` now also requires `IMAGE_SIGNING_PUBLIC_KEY`
  and `IMAGE_LOCAL_DIGEST` (fail fast). Public key only — never a signing
  secret in the agent or Matchmaker runtime.
- A long-running Matchmaker on `:8000` from before this phase will not mint
  `tag:host` keys or return `storage` on sessions. Restart uvicorn after
  pulling this commit, with `HEADSCALE_*` and `RENTERD_S3_*` set. Recreate
  Headscale so `acl.hujson` is mounted. Apply `alembic upgrade head`
  (`0006_user_storage_bucket`).
- Pause is still local `sharingEnabled=false` (no Matchmaker field). Skipping
  heartbeats looks like disconnect: after 45s the occupied session is
  interrupted and may rematch.
- Local Host Agent occupancy (in-session) is set on stub provision 200 and is
  **not** cleared when Matchmaker interrupts — there is no teardown HTTP port
  on the agent. Resume after pause can still show in-session if a guest already
  left.
- Provisioner is stubs only. Image verify is the software contract (small
  fixture bytes in tests). On macOS, a real Host Agent `POST /v1/provision`
  is 400 (`UnsupportedHostOs`) even with a valid signature; Matchmaker then
  keeps the seeded `kasmUrl`.
- Intent text stays in the harness. It is not persisted and is not on the
  session JSON. Keyword mapping is a smallest-slice heuristic (office/code →
  Standard; GPU/game/3D/training → Pro).
- Ranking uses declared caps on the host row. There is no measured latency or
  reliability score. A concurrent race on the same top-ranked host is still
  201+409 (no re-select of the runner-up on IntegrityError).
- Rematch is one `HostSelector.select`. If none is free, the user has no
  active session; the next book is 409 until a fresh approved host exists.
  Reclaim runs before a new book, so a displaced user can take the last
  remaining live host. The renterd bucket is per-user and is not re-created
  on rematch.
- **Real host payouts and user card charges are deferred.** The ledger records
  credits only. Do not add Stripe/PayPal.
- **Tailscale SaaS is not used.** Self-hosted Headscale mint+join is in.
  Default-deny ACLs are `headscale/acl.hujson`. Mint tags **`tag:host` only**
  — not per-session / ephemeral guest or user identities.
- **Architecture §9 item 5 — runbook exists, not auto-applied.** Opt-in
  `headscale/docker-compose.exit-node.yml` plus numbered steps in
  `docs/runbooks/headscale.md`. Host Agent join never advertises
  `--advertise-exit-node` and never sets `--exit-node`.
- **FR4/FR6 live mount is still paper.** Control-plane grant + compose exist.
  rclone inside a booted guest needs GATE-* + guest image. Session keys must
  not land on the volunteer host disk.
- Consumer GPU EULAs and ToS/liability still need legal review (architecture
  §9 items 10–11).
- Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
  `/aikido:setup`.

## Notes for next agent/session

- You are starting **phase 16** (Pro/Gamer). Read `PHASE_PLAN.md`, this file,
  and `docs/phase-logs/phase-15-renterd.md`. Do not start Storj. Do not close
  GATE-*. Do not treat renterd compose-up as a hardware-gate pass.
- Session router uses `HostSelector` + `HostProvisioner` via Depends. Do not
  import `RankedHostSelector` in `sessions.py` or `silence.py`. Do not put
  Firecracker/QEMU/WHPX names there. Occupancy stays on session indexes.
- Storage: Matchmaker talks path-style S3 to renterd (`HttpRenterdClient`).
  Tests swap `get_renterd_client()`. Never point pytest at a live Sia
  network. Guest rclone uses `--s3-env-auth` so secrets stay out of argv.
- Image verify: signed message is the raw 32-byte SHA-256 digest; signature is
  standard base64 Ed25519; public key is 64 hex chars. Matchmaker does not
  hold the private key. Tests use a documented fixture seed in Rust only.
- Mesh: Matchmaker holds `HEADSCALE_API_KEY`; the agent never does. Join uses
  `HEADSCALE_LOGIN_SERVER` from agent env plus a minted preauth key tagged
  `tag:host`. Unit tests mock Headscale HTTP. Headscale 0.26.1 wants a numeric
  `HEADSCALE_USER`. Policy expects the Headscale user named `dcl` (`dcl@`).
- Metering is `ledger.settle_ended_session` at user end and on silence
  interrupt. Unique `(session_id, party)` prevents double-credit. Rate is
  `CREDITS_PER_MINUTE`.
- Stale threshold is **45 seconds**. Agent ping interval is **10 seconds**.
  Never-seen hosts are not selectable. Heartbeat stamps `lastSeenAt` **before**
  reclaim so a late ping keeps that host's own session.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.”
