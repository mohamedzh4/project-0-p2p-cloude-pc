# Handoff — phase 15 renterd storage plane (2026-09-05)

- Phase goal: persistent user files via self-hosted Sia **renterd** S3
  (not Storj). Chat said “phase 14 renterd”; Headscale already occupied 14 —
  this is execution **phase 15**. Matchmaker issues a per-user bucket and
  returns guest rclone **locators** (no access keys); Host Agent does not
  mount on the volunteer OS.
- Verified git state at start: `eb73c52` on `main`, working tree dirty from
  an interrupted 15a/15b mix (boto3 vs httpx). Unified on httpx SigV4.
  Did not push. GATE-FC / GATE-WHPX / GATE-PRO remain OPEN. No Firecracker.
- Files created/changed:
  - `renterd/docker-compose.yml`, `renterd/.env.example`
  - `matchmaker/src/matchmaker/renterd.py`, `storage.py`, `config.py`,
    `deps.py`, `schemas.py`, `provision.py`
  - `matchmaker/src/matchmaker/routers/storage.py`, `routers/sessions.py`
  - `matchmaker/alembic/versions/0005_user_storage.py`,
    `0006_user_storage_bucket.py`
  - `matchmaker/tests/test_renterd.py`, `test_config.py`, `conftest.py`,
    `helpers.py`, `test_provision.py`, `test_sessions.py`
  - `matchmaker/.env.example`, `matchmaker/README.md`, `pyproject.toml`
  - `host-agent/src-tauri/src/storage.rs`, `provision_http.rs`, `lib.rs`
  - `host-agent/README.md`
  - `web/src/types.ts`, `web/src/components/SessionPanel.tsx`
  - `docs/runbooks/renterd.md`
  - `docs/phase-logs/phase-15-renterd.md` (this file)
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md` (short implemented note)
  - `.gitignore` (`renterd/data/`)
- Key decisions made and why:
  - **renterd, not Storj** (human 2026-09-05). Image
    `ghcr.io/siafoundation/renterd:2.9.3`. Required compose env
    `RENTERD_SEED` / `RENTERD_API_PASSWORD` (fail if missing). Lab network
    `zen`. S3 bind `RENTERD_S3_ADDRESS=0.0.0.0:8080` (upstream default is
    localhost). Host S3 port **8081** so Headscale keeps 8080. HTTP
    `--http :9980`.
  - **S3 v4 keypairs via bus API**, not compose YAML (`keypairsV4` removed
    in renterd 2.9). Operator PUT `/api/bus/settings/s3`.
  - **Per-user bucket** `dcl-<uuid>`, prefix `files/`. Unique constraint is
    the bucket, not the shared prefix (0006 repairs an earlier unique-prefix
    mistake).
  - **httpx SigV4 client**, not boto3. Path-style CreateBucket/PutObject.
    Redirects refused. Tests use `httpx.MockTransport` / `FakeRenterd`. No
    live Sia in pytest.
  - **Provision storage before occupy.** 502 `renterd storage provision
    failed` does not take the host. Silence rematch does not call renterd;
    the same bucket follows the user.
  - **rclone inside the guest.** Locators (bucket/prefix/endpoint/rclone) are
    on session JSON and `/v1/storage`. Access keys stay in Matchmaker/guest
    env (`S3_*`, aliases `RENTERD_S3_*`) — not in JSON. rclone uses
    `--s3-env-auth`. Host Agent `storageEndpoint` is URL-only and is ignored
    by the stub. `GuestRcloneMount` plans argv from public locators only.
    Live mount is **runbook + stub**, not auto-applied.
  - **S3_* names** (human spec) with `RENTERD_S3_*` aliases so namespaced
    env still works. Missing both fails fast as `S3_* is required`.
  - **POST /v1/storage** issues (201/200); **GET** lists objects or 404.
    `ListObjectsV2` is mocked. Marker key is `files/.dcl/provisioned`.
  - **Did not** change `HostSelector.select`, occupancy indexes, image
    signing, GPU exclusivity, OS split, CSP, `freezePrototype`, Argon2id,
    JWT, or Headscale/ACL/exit-node honesty. Did not close GATE-*.
- How to apply: numbered steps in `docs/runbooks/renterd.md` (compose up →
  bus v4 keypairs → Matchmaker `S3_*` (or `RENTERD_S3_*`) → `alembic upgrade head` →
  `POST` then `GET /v1/storage`). Restart uvicorn so settings load. Never commit `.env`
  or seeds.
- Key env names:
  - renterd compose: `RENTERD_SEED`, `RENTERD_API_PASSWORD` (required),
    `RENTERD_NETWORK` (default zen)
  - Matchmaker (startup): `S3_ENDPOINT`, `S3_ACCESS_KEY`,
    `S3_SECRET_KEY` (≥16 chars). Aliases `RENTERD_S3_*`. Optional `S3_REGION`,
    `S3_GUEST_ENDPOINT`
- Test results:
  - Matchmaker: **145 passed**, 0 failed. Was 126 before this phase. Added mocked
    CreateBucket/PutObject/ListObjectsV2, 409=exists, no redirects, fail-fast
    `S3_*` env (with `RENTERD_S3_*` aliases), POST issue + GET list/404, session
    locators without secrets, 502 without occupy, rclone without secrets, compose
    wiring, two-user distinct buckets, provision `storageEndpoint` without
    secrets.
  - Host Agent: **121 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 116. Added guest rclone plan stub (no secrets) and
    `storageEndpoint` accepted without host mount.
  - Web: **27 passed**, 0 failed.
- What’s built / verified / needs hardware / still paper:
  - **Built:** compose + Matchmaker S3 client + per-user bucket + guest
    rclone grant + Host Agent storageEndpoint stub.
  - **Verified:** pytest/cargo/vitest counts above. No live renterd/Sia
    smoke in this session.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO unchanged.
  - **Still paper:** rclone inside a booted guest; “saved as of HH:MM”;
    live Sia contracts/hosts on zen.
- Flagged for the next agent:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** No `gate-*-result.md`.
  - Next software phase is **16** (Pro/Gamer). Do not implement Storj. Do
    not treat compose-up as closing a gate.
  - Restart Matchmaker with `S3_*` (or `RENTERD_S3_*`) set. `alembic upgrade head`.
    Do not commit `.env` or seeds.
  - Aikido MCP `namespaceStatus: error`; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; working tree
  should be clean; still ahead of `origin/main`; did not push.
