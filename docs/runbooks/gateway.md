# Public HTTPS gateway — operator runbook

**Implemented in code, not yet run across real networks.** Do not write
“confirmed over the internet” until a human reports a host and a client on
genuinely different networks.

This is the Network-plane ingress that makes **zero-install Standard Lab
(browser only)** possible. Kasm/Sunshine locators stay Headscale mesh
addresses on the host row. Matchmaker returns `https://<your-domain>/s/<token>/`
when `GATEWAY_PUBLIC_ORIGIN` is set. The gateway joins the tailnet as
`tag:gateway` and reverse-proxies that token to one session.

Phase 34 (TAP/guest net) is separate and still required on the host machine.

## What I need from you before a real deploy

Do not invent these. Confirm before DNS or compose:

1. **Domain (or subdomain)** you control DNS for, e.g. `lab.example.com`.
   I will not pick one. After you name it, we put an A/AAAA record on the
   gateway VPS and set `GATEWAY_DOMAIN` / `GATEWAY_PUBLIC_ORIGIN`.
2. **Where the gateway runs:** a Linux VPS you operate. Same box as the
   platform exit node (`dcl-platform-exit`), or a separate VPS — say which
   you prefer.
3. **Sia mainnet** is a different decision (`docs/runbooks/sia-mainnet.md`).
   Do not switch `RENTERD_NETWORK` until you fund a wallet.

## What this does / does not do

**Does:** Caddy terminates TLS (Let’s Encrypt/ACME); gateway listens on
loopback; Matchmaker mints a scoped, unguessable token (architecture §9
item 9 — short-lived, scoped to one session; reused for that session’s
HTTP/WebSocket so the web UI can load); wrong/expired tokens are 404 with
no default backend; failed lookups are rate-limited and logged (token
hash prefix only).

**Does not:**

- Pairwise `tag:session-<uuid>` ACLs. **Headscale 0.26 still cannot mint
  pairwise owners.** `tag:gateway` ↔ `tag:guest` is role-based: every
  gateway node may reach every guest, renters themselves are not on the mesh. The
  token mapping is what stops the gateway from proxying a session it was
  not scoped for. The process must only dial the resolved URL.
- Grant `tag:gateway` → `tag:host`, `tag:matchmaker`,
  `tag:exit`, or `autogroup:internet`. Public HTTPS is the VPS NIC, not a
  mesh exit.
- Moonlight (Pro native client). `sunshineUrl` is wrapped; `moonlight.host`
  stays the mesh hostname.

## ACL grants (`headscale/acl.hujson`)

| Allow | From | To |
| --- | --- | --- |
| Public ingress | `tag:gateway` | `tag:guest` (and reverse) |

Reload Headscale after pulling this policy.

## 1. DNS (after you name the domain)

Create an A (and AAAA if you have IPv6) record for that name pointing at
the gateway VPS. Ports 80 and 443 must reach Caddy.

## 2. Mint `tag:gateway` (CLI, not Matchmaker)

From `headscale/` (same pattern as the exit-node key):

```bash
docker compose exec headscale headscale preauthkeys create --user 1 --tags tag:gateway
```

Never reuse a Host Agent, exit-node, or session guest/user preauth.

## 3. Matchmaker env

In `matchmaker/.env` (never commit):

```
GATEWAY_PUBLIC_ORIGIN=https://<your-domain>
GATEWAY_RESOLVE_TOKEN=<long random secret, ≥16 chars>
GATEWAY_TOKEN_TTL_SECONDS=43200
```

Restart uvicorn. Pytest pops these so a local `.env` cannot wrap tests.

## 4. Gateway compose (Linux VPS you operate)

```bash
cp gateway/.env.example gateway/.env
```

Set `GATEWAY_DOMAIN`, `GATEWAY_ACME_EMAIL`, `GATEWAY_PUBLIC_ORIGIN`,
`GATEWAY_RESOLVE_TOKEN` (same as Matchmaker), `MATCHMAKER_URL`,
`HEADSCALE_LOGIN_SERVER`, `HEADSCALE_GATEWAY_AUTHKEY`.

```bash
cd gateway
docker compose up -d
```

Caddy obtains a Let’s Encrypt certificate for `GATEWAY_DOMAIN`. The
gateway process binds `127.0.0.1:7843` only.

## 5. Smoke (software)

Book a session. `connection.kasmUrl` / `sunshineUrl` should be
`https://<domain>/s/<token>/`, not a `100.x` / MagicDNS mesh name.
Opening that URL from a browser **without** Tailscale installed is the
human cross-network test — not claimed by this runbook.

Wrong token → 404. Ended session → 404.

## 6. How to report back

Record (no secrets, no “confirmed over the internet” unless true):

1. Domain you used and that you control its DNS.
2. Same VPS as the exit node, or separate.
3. Browser without Tailscale opened the gateway URL (or what failed).
4. Host and client networks/cities if you are closing that claim.
