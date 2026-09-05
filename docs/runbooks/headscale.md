# Headscale (self-hosted) — operator runbook

**This is not Tailscale SaaS.** There is no Tailscale cloud account. The
control server is Headscale in `headscale/docker-compose.yml`.

Hardware gates (GATE-FC / GATE-WHPX / GATE-PRO) stay **OPEN**. Joining the
mesh does not boot a guest.

## What this does / does not do

**Does:** run Headscale locally; mint a Host Agent preauth key through the
Matchmaker; join that agent with `tailscale up --login-server` from Rust.

**Does not (phase 14c — still paper):**

- Default-deny ACLs or ephemeral **per-session** tagged identities
  (architecture §9 item 4). `headscale/config.yaml` leaves `policy.path`
  empty, so nodes can talk to each other. That is not production isolation.
- Guest internet egress via **platform-operated exit nodes** (architecture
  §9 item 5). Volunteer hosts must **not** advertise `--advertise-exit-node`.
  Guest traffic that reaches the internet today would still egress through
  the host path until 14c. Do not treat mesh join as solving the legal
  exposure of a volunteer's home IP.

## 1. Copy Headscale env and start compose

From the repo root:

```bash
cp headscale/.env.example headscale/.env
```

`HEADSCALE_SERVER_URL` is required. Compose fails if it is missing (same
pattern as `matchmaker/docker-compose.yml` Postgres). Keep it equal to
`server_url` in `headscale/config.yaml` (local lab: `http://127.0.0.1:8080`).

```bash
cd headscale
docker compose up -d
```

## 2. Create a Headscale user

Headscale 0.26.1 mints preauth keys against a **numeric user id**.

```bash
docker compose exec headscale headscale users create dcl
docker compose exec headscale headscale users list
```

Note the id (often `1` on a fresh database). That value is `HEADSCALE_USER`
in Matchmaker `.env`.

## 3. Create an API key and put it in Matchmaker `.env`

```bash
docker compose exec headscale headscale apikeys create
```

Copy the printed key once. It cannot be retrieved again.

In `matchmaker/.env` (never commit this file):

```
HEADSCALE_URL=http://127.0.0.1:8080
HEADSCALE_LOGIN_SERVER=http://127.0.0.1:8080
HEADSCALE_API_KEY=<paste the apikeys create output>
HEADSCALE_USER=1
```

Restart uvicorn so Matchmaker reloads env. Missing Headscale vars fail fast
at process start.

## 4. Mint a preauth key (Matchmaker)

The Host Agent must already be registered (`POST /v1/hosts`) so it has a
`hostId`. Pending hosts may join.

```bash
curl -s -X POST "http://127.0.0.1:8000/v1/hosts/HOST_UUID/mesh/preauth" \
  -H "Authorization: Bearer $HOST_BOOTSTRAP_TOKEN"
```

Response (camelCase): `{ loginServer, preauthKey, user, reusable, ephemeral, expiresAt }`.
Treat `preauthKey` as a secret. Do not log it.

## 5. Host Agent join (Rust, not the webview)

In `host-agent/.env`:

```
MATCHMAKER_URL=http://127.0.0.1:8000
HOST_BOOTSTRAP_TOKEN=<same as Matchmaker>
HEADSCALE_LOGIN_SERVER=http://127.0.0.1:8080
```

Install the Tailscale client so `tailscale` is on `PATH` (or set
`TAILSCALE_BIN`). After register, if `HEADSCALE_LOGIN_SERVER` is set, the
agent POSTs `/v1/hosts/{id}/mesh/preauth` from Rust and runs:

`tailscale up --login-server $HEADSCALE_LOGIN_SERVER --authkey <minted> --accept-dns=false --accept-routes=false`

It never sends `--advertise-exit-node`. Missing Matchmaker URL, bootstrap
token, or `HEADSCALE_LOGIN_SERVER` fails fast (no hardcoded keys).

## 6. Platform exit nodes (NOT done — 14c)

Guest egress must leave through IPs **you** operate, not the volunteer's ISP
(architecture §9 item 5). That needs a small fleet of nodes you control,
advertised as exit nodes, plus ACL/route policy so **guests** use them and
**hosts** do not become exits.

When you are ready to operate one (Linux VM you own, not a volunteer PC):

1. Join that VM to this Headscale with a key tagged for infra (tags are not
   automated in Matchmaker yet).
2. On **that** VM only: `tailscale up --advertise-exit-node` (plus Headscale
   approval of the route). Never run that flag on a Host Agent.
3. Force guest nodes (when they exist) to `--exit-node=<platform-node>`.
4. Record the operator IPs and keep them off volunteer hardware.

Until those steps are in code and ACL-enforced, assume guest internet still
exits via the host path. See `PROJECT_STATUS.md` item **14c**.
