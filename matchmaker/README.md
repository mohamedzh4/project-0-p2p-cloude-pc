# Matchmaker

Control-plane service for the Distributed Compute Lab: auth, a single seeded
host, and session occupancy. This is Phase 1 — one manually approved host, no
matching algorithm, no billing, no VM provisioning, no Tailscale.

The Host Agent is a separate app and is not called from here.

## What it does

- Signup / login with Argon2id password hashes and JWT access tokens
- `POST /v1/sessions` assigns the logged-in user to the seeded host if it is free
  and returns the Kasm URL (and optional token) of a session that is already
  running on that host
- `POST /v1/sessions/{id}/end` frees the host so another user can take it

Matching is isolated in `SingleApprovedHostSelector`. Swap that class later for
real ranking (latency, reliability, inventory). Comments in the session router
mark where provisioning, Headscale, Storj, and billing will plug in.

## Manual steps

### 1. Postgres

Docker is the expected path. From this directory:

```bash
cp .env.example .env
```

Edit `.env` and set `POSTGRES_PASSWORD` and `JWT_SECRET` to real values (a long
random string for the JWT secret). Keep `DATABASE_URL` / `TEST_DATABASE_URL` in
sync with the password.

```bash
docker compose up -d
```

Wait until the `postgres` service is healthy. Pytest creates `matchmaker_test`
on first run if it does not already exist.

You can point `DATABASE_URL` at any Postgres 16 instance instead of Docker; the
app will not start if that variable or `JWT_SECRET` is missing.

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

### 4. Seed the one host

Put the already-running Kasm session URL (from the Phase 0 host PC) in `.env`
as `SEED_HOST_KASM_URL`. `SEED_HOST_KASM_TOKEN` is optional. Then:

```bash
python -m matchmaker.seed
```

The command is idempotent: if a host row already exists, it updates connection
details so you can paste a new Kasm URL without SQL. Inventory fields
(`SEED_HOST_SHARED_*`) are optional; defaults are 4 cores / 8 GiB / no GPU.

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
| POST | `/v1/sessions` | Bearer | 201 new, 200 if you already have one, 409 if the host is taken |
| POST | `/v1/sessions/{id}/end` | Bearer | owner only; frees the host |

JSON is camelCase. Passwords are never returned.

## Out of scope (Phase 1)

- Real matching across many hosts
- Billing / credits
- Calling the Host Agent to boot a VM
- Tailscale / Headscale identities
- Storj mounts
- The user-facing web app
