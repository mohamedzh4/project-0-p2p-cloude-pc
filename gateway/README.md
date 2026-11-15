# Public HTTPS gateway (Network plane)

Reverse-proxies a browser on the public internet to a session's Kasm
(Standard) or Sunshine web UI (Pro) inside the Headscale tailnet. The browser
does not install Tailscale. Implemented in code, not yet run across real
networks.

Hardware / cross-city sessions are **not** confirmed. See `PROJECT_STATUS.md`.

## What this does / does not do

**Does:** listen on loopback; Caddy terminates TLS; resolve a scoped Matchmaker
token to one session locator; proxy HTTP (and WebSocket) to that locator only;
rate-limit failed lookups; join the tailnet as `tag:gateway`.

**Does not:**

- Invent a public domain. You supply DNS.
- Pairwise per-session ACLs. Headscale 0.26 still cannot mint
  `tag:session-<uuid>` owners. `tag:gateway` may reach every `tag:guest`
  (same leftover as `tag:user`). The token mapping is the session scope.
- Reach `tag:host` (no ACL grant). The process must only dial the resolved URL.
- Switch Sia to mainnet.
- Prove guest networking on **metal**. Phase 34 is a software contract
  (QEMU SLIRP); it is not a live desktop confirmation.

## Env

See `.env.example`. Matchmaker needs the same `GATEWAY_PUBLIC_ORIGIN` and
`GATEWAY_RESOLVE_TOKEN`. Operator steps: `docs/runbooks/gateway.md`.
