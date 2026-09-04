# Matchmaker

Control-plane service for the Distributed Compute Lab: auth, host inventory,
heartbeats, and session occupancy. Hosts register declared caps and start
**pending**; sessions book a manually approved host whose last heartbeat is
fresh (within 45s). Occupancy stays on session indexes, not a host flag. No
matching algorithm, no billing, no VM provisioning, no Tailscale.

The Host Agent is a separate app and is not called from here.

## What it does

- Signup / login with Argon2id password hashes and JWT access tokens
- `POST /v1/hosts` / `PUT /v1/hosts/{id}` register or update declared caps
  (`sharedLogicalCores`, `sharedMemoryBytes`, `shareGpu`, `sharedGpuId`,
  optional `kasmUrl`). New hosts are `pending`. GPU identity round-trips;
  sharing a GPU is exclusive for the session, not a concurrent slice.
- `POST /v1/hosts/{id}/heartbeat` (Host Agent bootstrap Bearer) stamps
  `lastSeenAt`. Register/update does **not** count as a ping. Never-seen or
  stale approved hosts are not booked.
- `POST /v1/hosts/{id}/approve` (or `python -m matchmaker.seed --approve-host-id`)
  is the manual approval path. The Host Agent bootstrap token cannot approve.
- `POST /v1/sessions` assigns the logged-in user to an approved host if it is
  free and returns the Kasm URL (and optional token) already stored on that host
- `POST /v1/sessions/{id}/end` frees the host so another user can take it

Matching is isolated in `SingleApprovedHostSelector` (`status == approved`,
fresh `last_seen_at`, and no active session). Staleness lives in the selector
(and helpers it calls), not the session router. Swap that class later for real
ranking (latency, reliability, inventory). Comments in the session router mark
where provisioning, Headscale, Storj, and billing will plug in.

## Manual steps

### 1. Postgres

Docker is the expected path. From this directory:

```bash
cp .env.example .env
```

Edit `.env` and set `POSTGRES_PASSWORD`, `JWT_SECRET`, `HOST_BOOTSTRAP_TOKEN`,
and `HOST_APPROVE_TOKEN` to real values (long random strings; the two host
tokens must differ). Keep `DATABASE_URL` / `TEST_DATABASE_URL` in sync with the
password.

```bash
docker compose up -d
```

Wait until the `postgres` service is healthy. Pytest creates `matchmaker_test`
on first run if it does not already exist.

You can point `DATABASE_URL` at any Postgres 16 instance instead of Docker; the
app will not start if `DATABASE_URL`, `JWT_SECRET`, `HOST_BOOTSTRAP_TOKEN`, or
`HOST_APPROVE_TOKEN` is missing.

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

Head includes `0002_host_last_seen` (`hosts.last_seen_at`, nullable). Do not
rewrite `0001_initial`.

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

## API

| Method | Path | Auth | Notes |
| --- | --- | --- | --- |
| POST | `/v1/auth/signup` | no | 201 + JWT |
| POST | `/v1/auth/login` | no | 200 + JWT |
| GET | `/v1/auth/me` | Bearer | `{ id, email }` |
| POST | `/v1/hosts` | Host bootstrap Bearer | 201 pending host |
| PUT | `/v1/hosts/{id}` | Host bootstrap Bearer | 200 update caps; 201 if that id is new; status unchanged |
| POST | `/v1/hosts/{id}/heartbeat` | Host bootstrap Bearer | 200 stamps `lastSeenAt`; 404 if unknown; does not approve |
| POST | `/v1/hosts/{id}/approve` | Approve Bearer | 200 `approved`; bootstrap token is rejected |
| POST | `/v1/sessions` | User Bearer | 201 new, 200 if you already have one, 409 if no approved host is free |
| POST | `/v1/sessions/{id}/end` | User Bearer | owner only; frees the host |

JSON is camelCase. Passwords and host tokens are never returned. Host inventory
does not include `kasmToken`. Host responses include `lastSeenAt` (null until a
heartbeat).

## Out of scope

- Real matching across many hosts
- Billing / credits
- Calling the Host Agent to boot a VM
- Interrupting an active session when heartbeats go silent
- Tailscale / Headscale identities
- Storj mounts
- The user-facing web app
