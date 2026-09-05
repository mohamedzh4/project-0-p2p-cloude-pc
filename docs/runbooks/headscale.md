# Headscale (self-hosted) — operator runbook

**This is not Tailscale SaaS.** There is no Tailscale cloud account. The
control server is Headscale in `headscale/docker-compose.yml`.

Hardware gates (GATE-FC / GATE-WHPX / GATE-PRO) stay **OPEN**. Joining the
mesh does not boot a guest.

## What this does / does not do

**Does:** run Headscale locally with a checked-in **default-deny** ACL file;
mint a Host Agent preauth key tagged `tag:host`; join that agent with
`tailscale up --login-server` from Rust; document a **platform** exit-node
path (opt-in compose overlay + steps below).

**Does not:**

- **Per-session** or ephemeral tailnet identities. Mint tags the **host
  node** (`tag:host`) only. `tag:guest` / `tag:user` / `tag:matchmaker` are
  reserved in the policy and are not minted on book.
- Auto-apply `--exit-node` on guests or hosts. Architecture §9 item 5 is
  **runbook exists, not auto-applied**. Volunteer hosts still must **not**
  advertise `--advertise-exit-node`. Until a guest is actually started with
  `--exit-node=<platform-node>`, guest internet can still egress via the
  host path.

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
```

Restart uvicorn so Matchmaker reloads env. Missing Headscale vars fail fast
at process start.

## 4. Mint a preauth key (Matchmaker)

The Host Agent must already be registered (`POST /v1/hosts`) so it has a
`hostId`. Pending hosts may join. The minted key is tagged **`tag:host`**
on the Headscale side (not returned on the JSON body).

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

## 6. Platform exit nodes (runbook exists, not auto-applied)

Guest egress must leave through IPs **you** operate, not the volunteer's ISP
(architecture §9 item 5). The Host Agent must never advertise an exit node.
Guests are **not** started with `--exit-node` by Matchmaker or the agent.

### 6.1 Mint a `tag:exit` key (CLI, not the host mint route)

From `headscale/`:

```bash
docker compose exec headscale headscale preauthkeys create --user 1 --tags tag:exit
```

Copy the key into `headscale/.env` as `HEADSCALE_EXIT_AUTHKEY`. Never reuse a
Host Agent preauth. Never commit `.env`.

### 6.2 Start the platform exit node

**Preferred — Linux VM you operate** (not a volunteer PC, not this Mac as a
stand-in for production):

```bash
sudo tailscale up \
  --login-server "$HEADSCALE_SERVER_URL" \
  --authkey "$HEADSCALE_EXIT_AUTHKEY" \
  --advertise-exit-node \
  --accept-dns=false
```

Enable IPv4/IPv6 forwarding on that VM (`net.ipv4.ip_forward=1`,
`net.ipv6.conf.all.forwarding=1`). `autoApprovers.exitNode` is `tag:exit`
in the policy, so Headscale should accept the route without a manual
`routes enable` when the node joined with that tag.

**Optional local overlay** (Linux operator host with `/dev/net/tun`;
`network_mode: host` so `http://127.0.0.1:8080` reaches Headscale). Docker
Desktop on macOS is not this path:

```bash
cd headscale
docker compose -f docker-compose.yml -f docker-compose.exit-node.yml up -d
```

### 6.3 Point guests at that node (manual, when a guest exists)

When a guest Tailscale client exists (GATE-FC / GATE-WHPX — still **OPEN**):

1. Read the platform node's Tailscale IP or MagicDNS name
   (`dcl-platform-exit` for the overlay).
2. On the **guest** only: `tailscale up ... --exit-node=<platform-node>`.
3. Never pass `--advertise-exit-node` or `--exit-node` on the Host Agent.

Until step 6.3 is actually run on a guest, assume guest internet still exits
via the host path. `PROJECT_STATUS.md` records this as **runbook exists, not
auto-applied**.
