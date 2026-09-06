# Project Status — last updated: 2026-09-06, after phase 19 hypervisor fetch

## Done

- Specs: architecture + cahier in `docs/` (OS-conditional Standard Lab, GPU
  exclusivity, four planes). On `main`.
- Hardware gates **OPEN — pending real-hardware verification**:
  `docs/runbooks/gate-fc.md`, `docs/runbooks/gate-whpx.md`,
  `docs/runbooks/gate-pro.md`. A 2026-09-05 operator PASS was later
  **retracted as unverified** (`docs/phase-logs/phase-gates-operator-pass.md`).
  No attached `gate-*-result.md` or live session URL. This Mac cannot host.
  `SEED_HOST_KASM_URL` remains `https://example.invalid/kasm`.
- Headscale operator runbook: `docs/runbooks/headscale.md` (ACL grants,
  session guest/user mint, platform exit-node guest path).
- renterd operator runbook: `docs/runbooks/renterd.md` (compose, S3 v4
  keypairs, guest rclone argv). **Implemented storage is renterd, not Storj
  (human decision 2026-09-05).**
- Execution phases 1–19 on local `main` (not pushed):
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
    may include storage locators (endpoint/bucket/prefix/region, no keys) and
    a guest-mesh hint (no preauth). The stub never rclone-mounts on the host
    OS and never `tailscale up`s as the guest. When locators are complete it
    returns `guestRclone` argv (`--s3-env-auth`). Pro HTTP 200 adds `streamer:
    sunshine`, stub `sunshineUrl`, and a Moonlight host hint (`gpuExclusive`
    true).     Standard stays `streamer: kasm`. **Phase 18:** `host_capabilities.rs`
    probes OS, Linux `/dev/kvm` r/w (path-injectable), Windows WHPX/Hyper-V as
    a stub enum on non-Windows, GPU list + second-display heuristic (iGPU or
    2+ GPUs), and Linux IOMMU sysfs (unknown off Linux). The window shows
    Standard/Pro **offered** in plain language and never asks which backend.
    **Phase 19:** `hypervisor.rs` locates or fetches Firecracker on Linux
    Standard (documented GitHub v1.16.1 tarball + published SHA-256) into a
    user cache; checksum mismatch fails fast and does not install; the binary
    is never executed. Windows Standard logs **would fetch QEMU** (19b silent
    install). macOS skips with a logged reason. Tests inject a fetcher /
    `file://` / loopback httptest (no live GitHub in CI). **159/159**
    `cargo test --lib` (2026-09-06).
  - Matchmaker: Argon2id + JWT-from-env; signup/login/me; host inventory
    pending → manual approve; heartbeats; occupancy partial unique indexes
    `sessions_one_active_per_host` / `sessions_one_active_per_user`;
    `HostSelector.select(db, *, lab_tier) -> Host | None` via
    `get_host_selector()` → `RankedHostSelector`. `HostProvisioner` after
    occupancy commit, now advertising `imageDigest` + `imageSignature` for the
    booked lab tier plus optional storage locators and a guest-mesh hint (no
    preauth, no S3 keys). `GET /v1/images/{labTier}` is checksum metadata
    only. `POST /v1/hosts/{id}/mesh/preauth` mints a Headscale preauth key
    tagged **`tag:host`**. `POST /v1/sessions` mints ephemeral **`tag:guest`**
    and **`tag:user`** keys (Headscale 0.26 `aclTags`) with unique hostnames
    and puts `--exit-node=$HEADSCALE_EXIT_NODE` on the guest join plan
    (default `dcl-platform-exit`). Session end / silence interrupt best-effort
    expires those hostnames. `POST`/`GET /v1/storage` and session JSON
    `storage` issue a per-user renterd S3 bucket (`dcl-<uuid>`) and guest
    rclone locators + `rcloneArgs` (**no access keys** on the wire;
    `--s3-env-auth`). `GET` is 404 until issued. Bucket provision on book runs
    **before** occupy; renterd or Headscale mint failure is 502 without taking
    the host. Session `connection` is Kasm for Standard and Sunshine/Moonlight
    for Pro (`gpuExclusive: true`) — not Kasm renamed. Locator still lives in
    `hosts.kasm_url`. Session end meters `startedAt` → `endedAt` into
    `ledger_entries`. Silence (~45s) on an occupied host marks the session
    `interrupted`, settles the ledger once, expires session mesh nodes, and
    rematches via one `select` (same renterd bucket follows the user; rematch
    does not call renterd). Required env includes `CREDITS_PER_MINUTE`, the
    four `IMAGE_*` pairs, `HEADSCALE_*` (optional `HEADSCALE_EXIT_NODE`,
    default `dcl-platform-exit`), and `S3_ENDPOINT` / `S3_ACCESS_KEY` /
    `S3_SECRET_KEY` (or `RENTERD_S3_*` aliases). **161/161** pytest
    (2026-09-06). Session JSON still has no `intent` field. No
    latency/reliability columns. No Matchmaker pause field. No payment
    processor. No image CDN / multi-GB download. No live Sia network in unit
    tests. No live Headscale in unit tests.
  - Web harness: signup/login, FR2 intent prompt → `labTier`, request/end
    session, session history + credits debited, optional storage locator
    (endpoint/bucket only — not secrets), mesh hostname/tags/exit-node (not
    the preauth secret). Pro copy points at Sunshine / Moonlight; Standard
    still opens Kasm in a new tab. **31/31** vitest; `npm run typecheck`
    passed.
  - Headscale compose in `headscale/` (image `0.26.1`) with checked-in
    default-deny `acl.hujson`. Guest-path exit-node overlay
    `docker-compose.exit-node.yml` (`TS_HOSTNAME: dcl-platform-exit`). renterd
    compose in `renterd/` (image `ghcr.io/siafoundation/renterd:2.9.3`, zen
    testnet, S3 on loopback **8081**). No Tailscale cloud account. No Storj.

## In Progress

- None. Numbered software phases 1–19 are on local `main`. Hardware gates are
  **OPEN — pending real-hardware verification**. Next numbered phase is 20
  (guest image fetch/verify). Do not mark gates closed. Windows QEMU silent
  install is leftover **19b**, not a substitute for 20.

## Up Next

**Phase 20 — Guest image fetch/verify** (`PHASE_PLAN.md`).
Do not invoke Firecracker/QEMU. Do not create `gate-*-result.md`. Do not
invent live URLs. Leftover paper (not a substitute for 20):

1. Pairwise mesh ACLs — Headscale 0.26 file policy cannot mint
   `tag:session-<uuid>` owners; every `tag:user` may still reach every
   `tag:guest`.
2. Live guest Tailscale/rclone — join/mount plans exist; no guest image
   in-repo to execute them. “Saved as of HH:MM” still needs that image.
3. FR13 latency ranking waits on a measured value — do not fake `latency_ms`.
4. **19b** Windows QEMU silent install — phase 19 only logs would-fetch.

## Known Risks / Open Items

- **GATE-FC / GATE-WHPX / GATE-PRO are OPEN — pending real-hardware
  verification.** A 2026-09-05 operator PASS was later retracted as unverified.
  No attached GATE result template or live session URL in this repo. This Mac
  still cannot host. `SEED_HOST_KASM_URL` remains `https://example.invalid/kasm`.
  Do not invent lab output. Do not mark gates closed.
- **Phase 15 race (2026-09-05):** `0a59f68` issued renterd S3 keys on session
  JSON; `9f69179` kept keys in env and locators-only on
  `POST`/`GET /v1/storage`. Phase 16–17 kept the locators-only contract.
  An unused `0007_host_latency_ms` draft from a parallel agent was **not**
  applied.
- Local `main` is ahead of `origin/main` (planning + phases 1–19). Nothing was
  pushed this session.
- Host register/heartbeat/provision/earnings/mesh-preauth auth uses a shared
  `HOST_BOOTSTRAP_TOKEN`. Per-host tokens are not in this MVP.
- Existing local `matchmaker/.env` must have `HOST_BOOTSTRAP_TOKEN` and
  `HOST_APPROVE_TOKEN` (different values), `CREDITS_PER_MINUTE`, the four
  `IMAGE_*` digest/signature pairs, `HEADSCALE_*`, and `S3_*` (or
  `RENTERD_S3_*` aliases) or uvicorn fails at startup. `HEADSCALE_EXIT_NODE`
  defaults to `dcl-platform-exit` if unset (`off` disables guest `--exit-node`).
  Pytest overwrites Headscale and renterd env to `.invalid` so a local `.env`
  cannot point tests at a live instance. Do not commit `.env`, renterd seeds,
  or `renterd/data/`. Copy fixture `IMAGE_*` values from `.env.example` for
  local stub verify; they are not a production key. Optional
  `STALE_SESSION_SWEEP_INTERVAL_SECONDS` defaults to 15.
- Enabling `HOST_AGENT_LISTEN_ADDR` now also requires `IMAGE_SIGNING_PUBLIC_KEY`
  and `IMAGE_LOCAL_DIGEST` (fail fast). Public key only — never a signing
  secret in the agent or Matchmaker runtime.
- A long-running Matchmaker on `:8000` from before this phase will not return
  `mesh` / `rcloneArgs` / `streamer` on sessions. Restart uvicorn after
  pulling this commit, with `HEADSCALE_*` and `S3_*` (or `RENTERD_S3_*`) set.
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
  keeps the seeded locator.
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
  on rematch. Rematch mints a new guest/user mesh for the new session id.
- **Real host payouts and user card charges are deferred.** The ledger records
  credits only. Do not add Stripe/PayPal.
- **Tailscale SaaS is not used.** Self-hosted Headscale mint+join is in.
  Default-deny ACLs are `headscale/acl.hujson`. Host mint is `tag:host`.
  Session mint is ephemeral `tag:guest` + `tag:user` with unique hostnames.
  ACL grants stay role-based (not pairwise).
- **Architecture §9 item 5 — software-applied on the guest join plan, not
  live-verified.** Guest `tailscaleUp` includes `--exit-node` from
  `HEADSCALE_EXIT_NODE` (default overlay hostname `dcl-platform-exit`). Host
  Agent join never advertises `--advertise-exit-node` and never sets
  `--exit-node`. Until a guest image actually runs those args against a
  running exit node, guest internet can still egress via the host path.
- **FR4/FR6 live mount is still paper.** Control-plane grant + argv/plan +
  provision `guestRclone` exist. rclone inside a booted guest still needs a
  guest image in-repo. Session keys must not land on the volunteer host disk.
- Consumer GPU EULAs and ToS/liability still need legal review (architecture
  §9 items 10–11).
- Aikido scan: MCP `namespaceStatus: error` this session (tool discovery
  failed). Not a scan pass. Run `/aikido:setup`.

## Notes for next agent/session

- Execution **phase 19 is done** (hypervisor binary fetch + log). Hardware
  gates stay **OPEN — pending real-hardware verification** — see
  `docs/phase-logs/phase-gates-operator-pass.md` (historical PASS + retraction)
  and `docs/phase-logs/phase-19-hypervisor-fetch.md`. Next is **phase 20**.
  Read `PHASE_PLAN.md` and this file. Do not start Storj. Do not invent lab
  output, a live Kasm URL, or `latency_ms`. Do not treat renterd compose-up as
  a live desktop. Do not weaken HostSelector, occupancy indexes, GPU
  exclusivity copy, OS-conditional stubs, or CSP. Do not mark gates closed.
  Do not fetch Firecracker from `cargo test` (injected fetcher / file:// /
  httptest). Windows QEMU silent install is **19b**.
- Session router uses `HostSelector` + `HostProvisioner` via Depends. Do not
  import `RankedHostSelector` in `sessions.py` or `silence.py`. Do not put
  Firecracker/QEMU/WHPX names there. Occupancy stays on session indexes.
- Storage: Matchmaker talks path-style S3 to renterd (`HttpRenterdClient`).
  Tests swap `get_renterd_client()`. Never point pytest at a live Sia
  network. Guest rclone uses `--s3-env-auth` so secrets stay out of argv.
  No access keys on `/v1/storage` or session JSON.
- Image verify: signed message is the raw 32-byte SHA-256 digest; signature is
  standard base64 Ed25519; public key is 64 hex chars. Matchmaker does not
  hold the private key. Tests use a documented fixture seed in Rust only.
- Mesh: Matchmaker holds `HEADSCALE_API_KEY`; the agent never does. Host join
  uses `HEADSCALE_LOGIN_SERVER` from agent env plus a minted preauth key tagged
  `tag:host`. Session JSON `mesh` holds guest/user keys. Unit tests mock
  Headscale HTTP. Headscale 0.26.1 wants a numeric `HEADSCALE_USER`. Policy
  expects the Headscale user named `dcl` (`dcl@`).
- Metering is `ledger.settle_ended_session` at user end and on silence
  interrupt. Unique `(session_id, party)` prevents double-credit. Rate is
  `CREDITS_PER_MINUTE`.
- Stale threshold is **45 seconds**. Agent ping interval is **10 seconds**.
  Never-seen hosts are not selectable. Heartbeat stamps `lastSeenAt` **before**
  reclaim so a late ping keeps that host's own session.
- Prefer this file over `PROJECT_CONTEXT.md` for “where we are.”
