# Handoff — phase 14c Headscale ACLs + platform exit-node runbook (2026-09-05)

- Phase goal: architecture §9 items 4–5 as a software contract — default-deny
  Headscale ACLs in-repo, host tagging on preauth mint, platform exit-node
  path that is not the volunteer Host Agent.
- Verified git state at start: `fd1ff8f` on `main`, working tree clean, branch
  ahead of `origin/main` by 15. Did not push. GATE-FC / GATE-WHPX / GATE-PRO
  remain OPEN. Did not start Storj. Did not integrate Tailscale SaaS.
- Files created/changed:
  - `headscale/acl.hujson` (new)
  - `headscale/docker-compose.exit-node.yml` (new)
  - `headscale/config.yaml`, `docker-compose.yml`, `.env.example`
  - `matchmaker/src/matchmaker/headscale.py`
  - `matchmaker/src/matchmaker/routers/hosts.py`, `routers/sessions.py`
  - `matchmaker/tests/test_headscale.py`
  - `matchmaker/.env.example`, `matchmaker/README.md`
  - `host-agent/src-tauri/src/mesh.rs`
  - `host-agent/.env.example`, `host-agent/README.md`
  - `docs/runbooks/headscale.md`
  - `docs/phase-logs/phase-14c-headscale-acls-exit-node.md` (this file)
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
- Key decisions made and why:
  - **Default-deny via checked-in `acl.hujson`.** Headscale 0.26.1 policy
    engine uses `acls`, not `grants`. `config.yaml` `policy.path` is
    `/etc/headscale/acl.hujson` (no longer empty). Compose bind-mounts it
    read-only.
  - **Exact grants:** Matchmaker coordination (`tag:host`/`tag:guest`/
    `tag:user` ↔ `tag:matchmaker`); assigned user ↔ VM (`tag:user` ↔
    `tag:guest`); guest egress via platform exit (`tag:guest` → `tag:exit`
    and `autogroup:internet`). **Not granted:** guest→host, guest↔guest,
    host↔host, host→internet-via-exit. `autoApprovers.exitNode` is
    `tag:exit` only. Operator user in policy is `dcl@` (runbook still
    `headscale users create dcl`).
  - **Host-tagged, not per-session.** Mint sends `aclTags`/`acl_tags`
    `["tag:host"]`. Keys stay single-use, non-ephemeral, 1h TTL. HTTP
    response shape unchanged (no tags on the wire). `tag:guest` /
    `tag:user` / `tag:matchmaker` are reserved in policy and are **not**
    minted on book. Do not call these per-session identities.
  - **Platform exit node is operator-manual.** Opt-in overlay
    `docker-compose.exit-node.yml` (`network_mode: host`, Linux operator
    host, `HEADSCALE_EXIT_AUTHKEY` from a CLI `tag:exit` mint). Numbered
    runbook also covers a Linux VM you operate. Host Agent still never
    passes `--advertise-exit-node`, `--exit-node`, or `--advertise-tags`.
    Guests are not configured to `--exit-node` in code. **§9 item 5:
    runbook exists, not auto-applied.**
  - **Did not** change `HostSelector.select`, occupancy indexes, image
    signing, GPU exclusivity, OS split, CSP, `freezePrototype`, Argon2id, or
    JWT. Did not start Storj. Did not close GATE-*.
- How to apply: numbered steps in `docs/runbooks/headscale.md` (compose up
  with ACL mount → user `dcl` → apikey → mint `tag:host` → agent join →
  optional §6 exit node). Restart Headscale so the new policy file is
  mounted. Restart uvicorn so mint sends tags.
- Test results:
  - Matchmaker: **126 passed**, 0 failed. Was 123. Added host-tag mint
    assertions, mint JSON contract unchanged, ACL file parse + grants,
    config/compose wiring (policy path + opt-in exit overlay).
  - Host Agent: **116 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Join still has no exit-node / advertise-tags flags.
  - Web: **27 passed**, 0 failed.
- What’s built / verified / needs hardware / still paper:
  - **Built:** default-deny ACL file; host tag on mint; exit-node overlay +
    runbook.
  - **Verified:** mocked mint still does not call live Headscale; Host Agent
    join still has no exit-node flag; policy file parses and encodes the
    grants above.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO unchanged.
  - **Still not done:** per-session/ephemeral guest and user identities;
    auto `--exit-node` on guests. Volunteer ISP exposure remains until a
    guest is actually started with `--exit-node=<platform-node>`.
- Flagged for the next agent:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** No `gate-*-result.md`.
  - Next software phase is **15** (Storj). **Needs a human Storj account —
    do not implement unattended.**
  - Restart Matchmaker with `HEADSCALE_*` set. Recreate Headscale so
    `acl.hujson` is mounted. Do not commit `.env`.
  - Aikido MCP `namespaceStatus: error`; not a scan pass. Run `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; working tree
  should be clean; still ahead of `origin/main`; did not push.
