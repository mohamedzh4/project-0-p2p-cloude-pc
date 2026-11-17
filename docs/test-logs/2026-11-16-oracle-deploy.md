# 2026-11-16: first deploy to Oracle Cloud

Server: Oracle Cloud, Morocco West, `VM.Standard.E5.Flex`, 4 vCPU, 24 GB,
Ubuntu 24.04.5, x86_64. Storage: Cloudflare R2, one bucket. Run from the
Mac with `scripts/vps-deploy.sh` (twice: first deploy, then a redeploy
with migration 0010).

## Seen working

| Check | Result |
| --- | --- |
| Deploy script reaches `DONE` | Yes, on the second attempt (see issue 22) |
| Postgres, Headscale, gateway, web, Caddy containers | Up |
| Matchmaker service | Active; `http://127.0.0.1:8000/openapi.json` 200, 20 routes |
| Headscale `/health` on the server | 200 |
| Web app on the server (`127.0.0.1:7844`) | 200 |
| Database migrations | 0001 to 0010 applied |
| Matchmaker on the server writes to R2 with a per-user key | PUT own prefix 200; PUT another user's prefix 403 |
| Desktop password against real KasmVNC 1.3.4 (throwaway container on the server) | No password 401, wrong 401, right 200; session VM without its password stays locked |
| DuckDNS | `test-projet0.duckdns.org` resolves to the server |

## Not working yet

| Check | Result |
| --- | --- |
| Ports 80 and 443 from the internet | Closed: Oracle Security List has no ingress rule. The server's own firewall allows them. |
| TLS certificates | Not issued (the challenge cannot reach the server) |
| Public web page, `/api/docs`, mesh health over HTTPS | Not reachable |
| Gateway joined to Headscale | No (it logs in through the public mesh address) |

## Not tested

A booking, a Host Agent, a guest image, the gateway across networks.

## Update 2026-11-17: public after the Oracle ingress rule

| Check | Result |
| --- | --- |
| Ports 80 and 443 from the internet | Open |
| TLS certificates (ZeroSSL, via Caddy) | Issued for all four names after a Caddy restart |
| `https://test-projet0.duckdns.org/` | 200 |
| `https://test-projet0.duckdns.org/api/docs` | 200 |
| `https://mesh.test-projet0.duckdns.org/health` | 200 |
| `https://desktop.test-projet0.duckdns.org/` | 404 without a session link (expected) |
| Gateway joined the mesh | Yes, `100.64.0.2` (an older `dcl-gateway` node from the first start is offline) |
