# Matchmaker

Control-plane service for the Distributed Compute Lab (software MVP after
execution phase 15): auth, host inventory, heartbeats, session occupancy,
an internal credits ledger, signed base-image **metadata**, Headscale
**preauth mint** for Host Agent mesh join (self-hosted; no Tailscale SaaS),
and **renterd** S3 bucket provision for guest mounts (not Storj).
Hosts register declared caps and start **pending**;
sessions book a manually approved host whose last heartbeat is fresh (within
45s). Among eligible hosts, `RankedHostSelector` prefers more declared CPU,
then more RAM, then host id. Pro bookings also require a dedicated GPU id.
After occupancy commit, the Matchmaker calls a Host Agent **provision port**
(HTTP, or a test double) for a stub connection URL. Occupancy stays on session
indexes, not a host flag. Ending a session meters `startedAt` → `endedAt` into
`ledger_entries` (user debit / host credit). A host silent for 45s with an
active session is marked `interrupted`, billed once, and rematched via
`HostSelector.select` when another eligible host is free. FR2 intent mapping
lives in `web/`; this API still takes `labTier`. No measured
latency/reliability scores, no payment processor, no real hypervisor.
Default-deny ACLs are a Headscale policy file; platform exit-node egress is
runbook-only (not auto-applied). rclone belongs **inside the guest**.

How Matchmaker finds the agent: `hosts.agent_url` from inventory, else optional
`HOST_AGENT_PROVISION_URL`. If neither is usable, the seeded/placeholder
`kasmUrl` on the host row is kept. OS/backend selection stays in Host Agent
Rust (`plan_provision`); this service does not name Firecracker/QEMU/WHPX.

## What it does

- Signup / login with Argon2id password hashes and JWT access tokens
- `POST /v1/hosts` / `PUT /v1/hosts/{id}` register or update declared caps
  (`sharedLogicalCores`, `sharedMemoryBytes`, `shareGpu`, `sharedGpuId`,
  optional `kasmUrl`, optional `agentUrl`). New hosts are `pending`. GPU
  identity round-trips; sharing a GPU is exclusive for the session, not a
  concurrent slice. `agentUrl` is the origin of the Host Agent provision
  listener. Omit it on PUT to leave a stored value alone.
- `POST /v1/hosts/{id}/heartbeat` (Host Agent bootstrap Bearer) stamps
  `lastSeenAt`. Register/update does **not** count as a ping. Never-seen or
  stale approved hosts are not booked. A live ping is stamped **before**
  silence reclaim, so a late heartbeat keeps that host's own session.
- `POST /v1/hosts/{id}/approve` (or `python -m matchmaker.seed --approve-host-id`)
  is the manual approval path. The Host Agent bootstrap token cannot approve.
- `POST /v1/sessions` assigns the logged-in user to an approved host if it is
  free, then calls the Host Agent provision port. `connection.kasmUrl` comes
  from that stub result when the agent is reachable; otherwise the URL already
  stored on the host row. Before booking, the router reclaims sessions on
  silent hosts (same as history and heartbeat) and provisions a **renterd**
  per-user bucket. Session JSON includes `storage` (S3 endpoint, bucket,
  keys, rclone command for the **guest**). Mount is not on the volunteer host.
- `POST /v1/sessions/{id}/end` frees the host so another user can take it and
  writes ledger rows (user debit, host credit) from session duration. Ending an
  already-`interrupted` session is idempotent and does not rename the status.
- `GET /v1/sessions` lists the caller's sessions and lifetime `creditsDebited`.
  Status is `active`, `ended` (user end), or `interrupted` (heartbeat silence).
- `GET /v1/hosts/{id}/earnings` (host bootstrap Bearer) returns
  `creditsAccrued` (not paid out)
- `GET /v1/images/{labTier}` returns the signed SHA-256 digest for `standard`
  or `pro`. Provision POSTs those fields to the Host Agent. Not a disk CDN.
- `GET /v1/storage` (user JWT) creates or reuses this user's renterd bucket
  and returns the same guest mount grant as session JSON (`storage`). rclone
  belongs inside the guest. Host Agent provision may include `storageEndpoint`
  (URL only — no S3 secrets).

Matching is isolated in `RankedHostSelector` (`status == approved`, matching
tier, fresh `last_seen_at`, no active session, then declared CPU/RAM). Pro
requires `shareGpu` plus a non-empty `sharedGpuId`. Staleness lives in the
selector (and helpers it calls), not the session router. Silence teardown and
rematch live in `silence.py` and call `HostSelector.select` only. Latency/reliability
scores wait until they can be measured — extend this class, do not inline
ranking in the session router. Provisioning is a `HostProvisioner`
injected via `get_host_provisioner()` — tests swap a fake; production POSTs
`/v1/provision` on the agent. `POST /v1/hosts/{id}/mesh/preauth` mints a
Host Agent Headscale key via `get_headscale_client()` (tests swap a fake;
no live Headscale in pytest). Host mint is tagged `tag:host`. Default-deny
ACLs live in `headscale/acl.hujson`. Per-session identities and auto
`--exit-node` on guests are not in this call. Metering is in `ledger.py`, called at user
end and on silence interrupt. renterd S3 is `get_renterd_client()` (tests swap
a fake; no live Sia network in pytest).

## Manual steps

### 1. Postgres

Docker is the expected path. From this directory:

```bash
cp .env.example .env
```

Edit `.env` and set `POSTGRES_PASSWORD`, `JWT_SECRET`, `HOST_BOOTSTRAP_TOKEN`,
`HOST_APPROVE_TOKEN`, `CREDITS_PER_MINUTE` (integer credits per billed
minute; `0` is allowed), the `IMAGE_*` digest/signature pairs, the
Headscale vars (`HEADSCALE_URL`, `HEADSCALE_LOGIN_SERVER`, `HEADSCALE_API_KEY`,
`HEADSCALE_USER`), and the renterd S3 vars (`S3_ENDPOINT` / `S3_ACCESS_KEY` /
`S3_SECRET_KEY`, or the `RENTERD_S3_*` aliases). Keep `DATABASE_URL` /
`TEST_DATABASE_URL` in sync with the password. See `docs/runbooks/headscale.md`
and `docs/runbooks/renterd.md`.

```bash
docker compose up -d
```

Wait until the `postgres` service is healthy. Pytest creates `matchmaker_test`
on first run if it does not already exist.

You can point `DATABASE_URL` at any Postgres 16 instance instead of Docker; the
app will not start if `DATABASE_URL`, `JWT_SECRET`, `HOST_BOOTSTRAP_TOKEN`,
`HOST_APPROVE_TOKEN`, `CREDITS_PER_MINUTE`, the four `IMAGE_*` registry
variables, the four `HEADSCALE_*` mesh variables, or the `S3_*` /
`RENTERD_S3_*` storage variables are missing.

### 2. Python

Python 3.12+ is required.

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
```

### 3. Migrate

```bash
alembic upgrade head
```

Head includes `0006_user_storage_bucket` (per-user renterd bucket). Do not rewrite
`0001_initial`, `0002_host_last_seen`, `0003_host_agent_url`, or
`0004_ledger_entries`.

### 4. Seed or approve a host

Put the already-running Kasm session URL (from the Phase 0 host PC) in `.env`
as `SEED_HOST_KASM_URL`. `SEED_HOST_KASM_TOKEN` is optional. Then:

```bash
python -m matchmaker.seed
```

The command upserts by `SEED_HOST_DISPLAY_NAME` (default `phase1-lab-1`) and
marks that row **approved**, stamping `last_seen_at` so the local harness can
book immediately. Re-run seed (or POST a heartbeat) if more than 45s have
passed — silence is treated as stale. Other hosts are left alone. Inventory
fields (`SEED_HOST_SHARED_*`) are optional; defaults are 4 cores / 8 GiB / no
GPU.

To approve a host that registered through the API:

```bash
python -m matchmaker.seed --approve-host-id HOST_UUID
```

`https://example.invalid/kasm` is only a placeholder. The web harness will not
offer "Open desktop" until this is a real `http(s)` URL on the host.

### 5. Run

```bash
uvicorn matchmaker.main:app --reload --app-dir src
```

API docs: http://127.0.0.1:8000/docs

```bash
# signup
curl -s http://127.0.0.1:8000/v1/auth/signup \
  -H 'Content-Type: application/json' \
  -d '{"email":"ada@example.com","password":"password123"}'

# request a session (paste the accessToken from signup)
curl -s http://127.0.0.1:8000/v1/sessions \
  -H 'Authorization: Bearer ACCESS_TOKEN' \
  -H 'Content-Type: application/json' \
  -d '{"labTier":"standard"}'
```

## Tests

Needs the test database in `TEST_DATABASE_URL` (created by compose).

```bash
pytest
```

Expect more than the phase-14c baseline of 126. Unit tests mock renterd S3
and Headscale HTTP; they do not start Docker or talk to a live Sia network.

## API

| Method | Path | Auth | Notes |
| --- | --- | --- | --- |
| POST | `/v1/auth/signup` | no | 201 + JWT |
| POST | `/v1/auth/login` | no | 200 + JWT |
| GET | `/v1/auth/me` | Bearer | `{ id, email }` |
| POST | `/v1/hosts` | Host bootstrap Bearer | 201 pending host |
| PUT | `/v1/hosts/{id}` | Host bootstrap Bearer | 200 update caps; 201 if that id is new; status unchanged |
| POST | `/v1/hosts/{id}/heartbeat` | Host bootstrap Bearer | 200 stamps `lastSeenAt`; 404 if unknown; does not approve. Then reclaims other silent occupied hosts. |
| POST | `/v1/hosts/{id}/approve` | Approve Bearer | 200 `approved`; bootstrap token is rejected |
| POST | `/v1/sessions` | User Bearer | 201 new, 200 if you already have one (including a silence rematch), 409 if no approved host is free. New bookings call the provision port after occupancy commit. Reclaims silent occupied hosts first. |
| POST | `/v1/sessions/{id}/end` | User Bearer | owner only; frees the host; meters duration into ledger rows. `status` is `ended` or already-`interrupted`. |
| GET | `/v1/sessions` | User Bearer | session history + lifetime `creditsDebited`; reclaims silent occupied hosts (may rematch) |
| GET | `/v1/hosts/{id}/earnings` | Host bootstrap Bearer | `{ hostId, creditsAccrued }` — accrued, not paid out |
| POST | `/v1/hosts/{id}/mesh/preauth` | Host bootstrap Bearer | `{ loginServer, preauthKey, user, reusable, ephemeral, expiresAt }`. Mints via Headscale HTTP with `tag:host`. 404 unknown host; 502 if Headscale mint fails. Not a per-session identity. |
| GET | `/v1/storage` | User Bearer | renterd S3 grant for this user (`endpoint`, `bucket`, keys, rclone). Mount **inside the guest**. 502 if renterd S3 fails. |
| GET | `/v1/images/{labTier}` | no | `{ labTier, digestSha256, signature, algorithm: ed25519, digestAlgorithm: sha256 }` for `standard` or `pro`. Checksums only; not a disk download. |

JSON is camelCase. Passwords and host tokens are never returned. Host inventory
does not include `kasmToken`. Host responses include `lastSeenAt` (null until a
heartbeat) and `agentUrl` (null until the agent advertises a listen origin).

Optional Matchmaker env: `HOST_AGENT_PROVISION_URL` (http/https origin, no path)
when the booked host has no `agentUrl`. Not required; missing means fallback to
the host row `kasmUrl`. Invalid non-empty values fail fast at startup.
`CREDITS_PER_MINUTE` is required (non-negative integer). It is a ledger rate,
not a payment-processor key. Optional
`STALE_SESSION_SWEEP_INTERVAL_SECONDS` (default 15, `0` disables the
background loop only). Invalid or negative values fail fast. Book, history,
and heartbeat paths still reclaim when the loop is off.

Image registry (required): `IMAGE_STANDARD_DIGEST`, `IMAGE_STANDARD_SIGNATURE`,
`IMAGE_PRO_DIGEST`, `IMAGE_PRO_SIGNATURE`. Digests are 64-char SHA-256 hex
(optional `sha256:` prefix). Signatures are standard base64 of a 64-byte
Ed25519 signature over the **raw 32-byte digest**. `.env.example` holds
local-dev fixture values (not a production key, not a private key). The Host
Agent verifies with `IMAGE_SIGNING_PUBLIC_KEY` before a stub boot.

Headscale (required at Matchmaker startup): `HEADSCALE_URL` (API origin),
`HEADSCALE_LOGIN_SERVER` (returned to agents as `--login-server`),
`HEADSCALE_API_KEY` (from `headscale apikeys create`), `HEADSCALE_USER`
(numeric id on Headscale 0.26.1). Placeholders only in `.env.example`.
Compose for Headscale lives in `headscale/` — see `docs/runbooks/headscale.md`.
Pytest never calls a live Headscale.

renterd S3 (required at Matchmaker startup): `S3_ENDPOINT` (Matchmaker
→ renterd S3 origin; lab default `http://127.0.0.1:8081`), `S3_ACCESS_KEY`,
`S3_SECRET_KEY` (≥16 chars). `RENTERD_S3_*` aliases are accepted.
Optional `S3_REGION` (default `us-east-1`) and `S3_GUEST_ENDPOINT` (what
guests rclone; defaults to `S3_ENDPOINT`). Compose lives in `renterd/` — see
`docs/runbooks/renterd.md`. Pytest never calls a live renterd or Sia network.
JSON locators never include access keys.

The Host Agent provision listener (not this process) is `POST /v1/provision`
with the host bootstrap Bearer. The JSON body includes `imageDigest` and
`imageSignature` from this registry, and may include `storageEndpoint` (URL
only). It is not fetched from the webview.

## Out of scope

- Real measured latency / reliability scoring (declared-cap ranking is in)
- Payment processors / host payouts (internal ledger only)
- Invoking Firecracker, QEMU, Kasm, Sunshine, or VFIO
- Per-session / ephemeral Headscale identities (host mint is `tag:host` only)
- Auto-applied `--exit-node` on guests (platform overlay + runbook exist; not applied)
- rclone inside a live guest (Host Agent plan stub + runbook; needs GATE-* + guest image)
- Storj
- The user-facing web app
