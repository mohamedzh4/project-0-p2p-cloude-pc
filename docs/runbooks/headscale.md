# Headscale (self-hosted) — operator runbook

**This is not Tailscale SaaS.** There is no Tailscale cloud account. The
control server is Headscale in `headscale/docker-compose.yml`.

Hardware is implemented in code, not yet run on physical hardware.
Joining the mesh does not boot a guest. `SEED_HOST_KASM_URL` stays
`https://example.invalid/kasm`. See `PROJECT_STATUS.md`.

## What this does / does not do

**Does:** run Headscale locally with a checked-in **default-deny** ACL file;
mint a Host Agent preauth key tagged `tag:host`; join that agent with
`tailscale up --login-server` from Rust; mint **ephemeral** `tag:guest` and
`tag:user` preauth keys on each session book (Headscale 0.26 `aclTags`); put
`--exit-node=$HEADSCALE_EXIT_NODE` on the **guest** join plan (default
hostname `dcl-platform-exit` from the compose overlay).

**Does not:**

- Pairwise per-session ACL tags. Headscale 0.26 file policy has no mint-time
  API to add `tag:session-<uuid>` to `tagOwners`. Grants stay role-based:
  every `tag:user` may reach every `tag:guest`. Unique **hostnames** and
  ephemeral keys are the per-booking identity.
- Live guest egress. Software points guests at the platform exit node; a
  guest image still has to run `tailscale up` with that plan. Until a guest
  actually joins, traffic does not flow.
- `--advertise-exit-node` or `--exit-node` on the Host Agent. Volunteer
  hosts must not become exits and must not hairpin guest internet.

## Exact ACL grants (`headscale/acl.hujson`)

Headscale 0.26.1 uses `acls` (not `grants`). Policy usernames need a trailing
`@`; create the Headscale user as `dcl` so `dcl@` matches.

| Allow | From | To |
| --- | --- | --- |
| Matchmaker coordination | `tag:host` | `tag:matchmaker` (and reverse) |
| Matchmaker coordination | `tag:guest` | `tag:matchmaker` (and reverse) |
| Matchmaker coordination | `tag:user` | `tag:matchmaker` (and reverse) |
| Assigned user ↔ VM | `tag:user` | `tag:guest` (and reverse) |
| Guest egress via platform exit | `tag:guest` | `tag:exit` and `autogroup:internet` |

**Denied** (no accept rule): guest → host, guest ↔ guest, host ↔ host,
host → `autogroup:internet`. Exit-node auto-approval is `tag:exit` only.

Reload Headscale after editing the policy (`docker compose restart headscale`
or SIGHUP). The file is bind-mounted read-only.

## 1. Copy Headscale env and start compose

From the repo root:

```bash
cp headscale/.env.example headscale/.env
```

`HEADSCALE_SERVER_URL` is required. Compose fails if it is missing (same
pattern as `matchmaker/docker-compose.yml` Postgres). Keep it equal to
`server_url` in `headscale/config.yaml` (local lab: `http://127.0.0.1:8080`).

Do **not** put a real `HEADSCALE_EXIT_AUTHKEY` in git. The placeholder in
`.env.example` is only for the opt-in overlay in §6.

```bash
cd headscale
docker compose up -d
```

## 2. Create a Headscale user

Headscale 0.26.1 mints preauth keys against a **numeric user id**. The ACL
file expects this user to be named **`dcl`**.

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
HEADSCALE_EXIT_NODE=dcl-platform-exit
```

Restart uvicorn so Matchmaker reloads env. Missing Headscale vars fail fast
at process start.

## 4. Mint a preauth key (Matchmaker)

The Host Agent must already be registered (`POST /v1/hosts`) so it has a
`hostId`. Pending hosts may join. The minted key is tagged **`tag:host`**
on the Headscale side (not returned on the JSON body). Guest/user keys are
minted on `POST /v1/sessions` (`mesh.guest` / `mesh.user`), not this route.

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

It never sends `--advertise-exit-node`, `--exit-node`, or `--advertise-tags`
(tags come from the preauth key). Missing Matchmaker URL, bootstrap token,
or `HEADSCALE_LOGIN_SERVER` fails fast (no hardcoded keys).

## 6. Platform exit nodes (guest-path overlay)

Guest egress must leave through IPs **you** operate, not the volunteer's ISP
(architecture §9 item 5). The Host Agent must never advertise an exit node.

Matchmaker guest join plans always include
`--exit-node=$HEADSCALE_EXIT_NODE` unless that env is `off`. The default
value is `dcl-platform-exit` (the overlay `TS_HOSTNAME`). That is
**software auto-applied on the guest argv**. It is not live egress until a
guest image runs those args against a running exit node.

### 6.1 Mint a `tag:exit` key (CLI, not the host or session mint routes)

From `headscale/`:

```bash
docker compose exec headscale headscale preauthkeys create --user 1 --tags tag:exit
```

Copy the key into `headscale/.env` as `HEADSCALE_EXIT_AUTHKEY`. Never reuse a
Host Agent or session guest/user preauth. Never commit `.env`.

### 6.2 Start the platform exit node

**Preferred — Linux VM you operate** (not a volunteer PC, not this Mac as a
stand-in for production):

```bash
sudo tailscale up \
  --login-server "$HEADSCALE_SERVER_URL" \
  --authkey "$HEADSCALE_EXIT_AUTHKEY" \
  --advertise-exit-node \
  --accept-dns=false \
  --hostname dcl-platform-exit
```

Enable IPv4/IPv6 forwarding on that VM (`net.ipv4.ip_forward=1`,
`net.ipv6.conf.all.forwarding=1`). `autoApprovers.exitNode` is `tag:exit`
in the policy, so Headscale should accept the route without a manual
`routes enable` when the node joined with that tag.

**Guest-path overlay** (Linux operator host with `/dev/net/tun`;
`network_mode: host` so `http://127.0.0.1:8080` reaches Headscale). Docker
Desktop on macOS is not this path:

```bash
cd headscale
docker compose -f docker-compose.yml -f docker-compose.exit-node.yml up -d
```

### 6.3 Guest join plan (Matchmaker session JSON)

`POST /v1/sessions` returns `mesh.guest.tailscaleUp` already containing
`--exit-node=dcl-platform-exit` (or `HEADSCALE_EXIT_NODE`). Inject that into
the **guest** only. `mesh.user.tailscaleUp` does **not** set `--exit-node`.

Never pass `--advertise-exit-node` or `--exit-node` on the Host Agent.

Until a guest image actually runs step 6.3, assume guest internet still
exits via the host path. Software-complete ≠ live desktop.
