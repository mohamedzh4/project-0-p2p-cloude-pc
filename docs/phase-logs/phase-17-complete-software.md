# Handoff — phase 17 remaining mesh/storage guest path (2026-09-05)

- Phase goal: Finish remaining software (session Headscale tags, guest
  `--exit-node` join plan, locators-only guest rclone argv) without a second
  host PC and without new paid accounts.
- Verified git state at start: `740febc` on `main` (operator-declared
  GATE-FC / GATE-WHPX / GATE-PRO closed). Did not invent hardware logs. Did
  not claim live Kasm in-repo. `SEED_HOST_KASM_URL` stays
  `https://example.invalid/kasm`. Did not push. Did not open PRs.
- Files created/changed:
  - `matchmaker/src/matchmaker/mesh.py` (session hostnames, mint/expire,
    guest `--exit-node` argv, provision hint without preauth)
  - `matchmaker/src/matchmaker/headscale.py`, `config.py`, `storage.py`,
    `provision.py`, `schemas.py`, `silence.py`, `main.py`,
    `routers/sessions.py`, `routers/hosts.py`
  - `matchmaker/tests/` (`conftest.py`, `helpers.py`, `test_headscale.py`,
    `test_config.py`, `test_provision.py`, `test_renterd.py`)
  - `matchmaker/.env.example`, `matchmaker/README.md`
  - `host-agent/src-tauri/src/storage.rs`, `provision_http.rs`, `mesh.rs`
  - `host-agent/.env.example`, `host-agent/README.md`
  - `web/src/types.ts`, `web/src/components/SessionPanel.tsx`, `web/README.md`
  - `headscale/acl.hujson`, `headscale/docker-compose.yml`,
    `headscale/docker-compose.exit-node.yml`, `headscale/.env.example`
  - `docs/runbooks/headscale.md`, `docs/runbooks/renterd.md`
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`,
    `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-17-complete-software.md` (this file)
- Key decisions made and why:
  - **Headscale 0.26 maximum real tagging.** Preauth mint accepts `aclTags`
    (`tag:guest` / `tag:user`, ephemeral, unique hostnames). File policy has
    no API to insert `tag:session-<uuid>` into `tagOwners`, so ACL grants
    stay role-based (every `tag:user` may still reach every `tag:guest`).
  - **Guest `--exit-node` on the join plan.** `HEADSCALE_EXIT_NODE` defaults
    to overlay hostname `dcl-platform-exit`. `off` omits it. Host Agent join
    never `--advertise-exit-node` or `--exit-node`. Architecture §9 item 5 is
    software-applied on argv, not live-verified (no guest image).
  - **Guest preauth stays on session JSON `mesh`, not provision POST.** The
    volunteer host must not receive the guest authkey. Provision `guestMesh`
    is login-server / hostname / exit node / tags only.
  - **Locators-only rclone.** `--s3-env-auth`; no S3 keys on `/v1/storage` or
    session JSON. Host Agent `guestRclone` from complete locators; never
    mounts on the volunteer OS; never hits live Sia in tests.
  - **HostSelector.select unchanged.** Occupancy stays partial unique indexes.
    GPU exclusivity, OS-conditional stubs, CSP, freezePrototype, Argon2id,
    JWT-from-env unchanged.
  - **Gates stay operator-declared closed (2026-09-05).** Not tests, not an
    in-repo lab.
- Test results:
  - Matchmaker: **161 passed**, 0 failed. Was 150. Added session guest/user
    mint, 502 without occupy on Headscale failure, guest `--exit-node` argv,
    locators-only `rcloneArgs`, provision mesh hint without secrets.
  - Host Agent: **126 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 123. Added locator `guestRclone` plan and env-auth argv.
  - Web: **31 passed**, 0 failed. `npm run typecheck` passed.
- What’s built / verified / needs hardware / still paper:
  - **Built:** session ephemeral `tag:guest`/`tag:user`; guest `--exit-node`
    join plan; locators-only rclone argv + Host Agent `guestRclone`.
  - **Verified:** pytest/cargo/vitest counts above. Harness lists mesh
    hostnames/tags/exit-node (not preauth). SessionPanel live-desktop path
    was not browser-exercised (Matchmaker not assumed running).
  - **Needs hardware:** none for this software slice. Gates remain
    **operator-declared closed**; this Mac still cannot host.
  - **Still paper:** pairwise session ACLs; live guest Tailscale/rclone (no
    guest image); FR13 `latency_ms`; Stripe/payouts.
- Flagged for the next agent:
  - No numbered software phase after 17. Leftover is paper listed in
    `PROJECT_STATUS.md` Up Next.
  - Do not add Storj, Tailscale SaaS, or Stripe. Do not fake Firecracker /
    QEMU / Sunshine on this Mac. Do not put S3 keys on JSON.
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed. Split-to-PRs was not
    approved.
- Verified git state at end: this commit on local `main`; still ahead of
  `origin/main`; did not push.

## Later note (2026-09-06)

Hardware-gate **current** wording was corrected to **OPEN — pending
real-hardware verification**. The 2026-09-05 operator PASS recorded in this
handoff was later retracted as unverified. See
`docs/phase-logs/phase-gates-operator-pass.md` (retraction note) and
`PROJECT_STATUS.md`. This phase-17 software slice is unchanged.

