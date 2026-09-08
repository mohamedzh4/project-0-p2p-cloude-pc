# Sia mainnet — operator checklist (do not run unprompted)

**Do not switch `RENTERD_NETWORK` away from `zen` until you explicitly
go-ahead and fund a wallet.** That is real Siacoin (real money). This file
is the decision checklist only.

Implemented storage is already **renterd on the Sia network**, not a
centralized bucket and not Storj. Per-user bucket `dcl-<uuid>`, prefix
`files/`, guest-only rclone, `--s3-env-auth`, no access keys on
`/v1/storage` or session JSON — that contract stays. Zen **proves** the
software path with testnet hosts; it does not use independent mainnet
operators.

`renterd/.env.example` keeps `RENTERD_NETWORK=zen`. Compose defaults to
zen. Pytest never talks to a live Sia network.

## What already matches the target architecture

- Provider on the wire: `renterd` (S3-compatible API in front of Sia).
- Matchmaker `HttpRenterdClient`: CreateBucket / PutObject, path-style.
- One bucket per user (`dcl-<uuid>`), prefix `files/`.
- Locators only on JSON; keys stay in Matchmaker/guest env.
- rclone inside the guest (`mountInsideGuest: true`), not on the volunteer
  host OS.
- Host Agent provision gets locators, never mounts, never sees S3 keys.

Live guest mount still needs a booted guest with a network (TAP leftover).

## What changes when you move to mainnet

Do these **in order**. Do not start from CI or this Mac as a stand-in.

1. **New wallet seed (recommended).** A zen seed is a testnet wallet. Use a
   new 12-word seed for mainnet; store it offline. Never commit it. Never
   reuse the zen seed on mainnet.
2. **Stop zen compose** and take a backup of `renterd` data if you care
   about test objects. Mainnet consensus does not share zen chain state.
   A fresh volume is the safe default.
3. **Set `RENTERD_NETWORK=mainnet`** in `renterd/.env` (renterd’s own
   default is mainnet if unset; this repo currently forces zen). Do not
   commit the real `.env`.
4. **Sync consensus.** First start downloads the Sia mainnet chain. That
   can take hours (or longer) without a bootstrap/checkpoint. Instant
   sync (`renterd --instant`) is experimental and requires a v2-only
   wallet — treat it as optional, not required.
5. **Confirm sync** before expecting uploads:
   - UI on `http://127.0.0.1:9980` (SSH tunnel on a VPS).
   - `GET /api/bus/consensus/state` (or the UI consensus panel) shows
     synced.
6. **Fund the renterd wallet with real Siacoin (SC).**
   - `GET /api/bus/wallet` → address + balance (hastings; 1 SC = 10^24
     hastings).
   - Send SC from an exchange or wallet **you** control to that address.
   - Rule of thumb from Sia/renterd docs: keep **at least ~2× the
     autopilot allowance** in the wallet so contract formation and
     renewals are not starved.
   - Confirm `GET /api/bus/wallet` balance is non-zero and confirmed.
7. **Set autopilot allowance / storage target** in the renterd UI
   (Autopilot). This is the budget renterd may lock into contracts. Pick
   size from expected persistent-file volume plus redundancy, not from
   this checklist inventing a TB number.
8. **Wait for storage contracts to form** with independent mainnet hosts.
   Autopilot forms them after the node is synced and the wallet is
   funded. Expect **minutes to many hours**, not seconds. Default
   contract duration on Sia is on the order of **~3 months**, then
   renewals. Check `GET /api/bus/contracts` (or the Contracts tab) until
   you have a healthy set (docs/workshop examples often aim for ~10;
   production may want more — follow current renterd UI guidance).
9. **Re-PUT S3 v4 keypairs** if this is a new data volume
   (`docs/runbooks/renterd.md` §3). Matchmaker `S3_*` / `RENTERD_S3_*`
   stay locators-only on the API.
10. **Smoke Matchmaker** `POST`/`GET /v1/storage` still returns
    `provider: renterd`, `mountInsideGuest: true`, no access keys. A
    real object upload through rclone inside a guest is a later live
    test — not this checklist.

## Costs (order of magnitude, not a quote)

- **Contract formation:** small SC per contract (Sia docs: typically a
  handful of SC each, plus collateral locked for the term).
- **Storage + egress:** paid to hosts over the contract term; you only
  spend what you use, but the allowance is reserved up front.
- **SC market price** moves. Convert SC→fiat yourself before funding.
- **Wallet buffer:** extra SC beyond the allowance for renewals and
  host churn.

This repo will not buy SC, pick an exchange, or broadcast a mainnet
transaction.

## What does *not* change

- Bucket naming, prefix, rclone argv, `--s3-env-auth`.
- Guest-only mount; no keys on the wire.
- Pytest still mocks renterd HTTP and must never point at mainnet.
- Do not set `RENTERD_NETWORK=mainnet` in `.env.example` or CI.

## Go-ahead gate

Switch compose off zen only after you reply with:

1. You control a funded mainnet wallet (or just funded this renterd
   address).
2. You accept contract formation time and SC spend.
3. Explicit “switch renterd to mainnet” — not implied by deploying the
   public gateway.
