# renterd (Sia) — operator runbook

**This is not Storj.** Persistent files use a self-hosted Sia
[`renterd`](https://github.com/SiaFoundation/renterd) S3 gateway
(`ghcr.io/siafoundation/renterd:2.9.3`). rclone belongs **inside the guest**,
never on the volunteer host OS (architecture §8).

Hardware is implemented in code, not yet run on physical hardware.
Bringing up renterd does not boot a guest. See `PROJECT_STATUS.md`.
`SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.

Implemented storage (human decision 2026-09-05): **renterd, not Storj.**
Compose stays on the **zen testnet** (`RENTERD_NETWORK=zen`). Do not
switch to mainnet until you fund a wallet — checklist only:
`docs/runbooks/sia-mainnet.md`.

## What this does / does not do

**Does:** run renterd locally with a required seed and API password; expose
the S3-compatible API; let you set SigV4 keypairs via the bus API; tell
Matchmaker how to CreateBucket / PutObject and hand a guest rclone command.

**Does not:**

- Mount user data on the volunteer host. Host Agent `storageEndpoint` is a
  URL the **guest** should use. This agent never runs rclone.
- Talk to the public Sia network from pytest. Unit tests mock S3 HTTP.
- Show “saved as of HH:MM” in a live desktop (needs a guest image in-repo).
- Use Storj.

renterd 2.9 no longer reads `RENTERD_S3_KEYPAIRS_V4` / YAML `keypairsV4`.
S3 keys live in bus settings (`PUT /api/bus/settings/s3`).

S3 host port defaults to **8081** so it does not collide with Headscale’s
8080. HTTP API stays on 9980 (`--http :9980`, renterd 2.9 default).

## 1. Copy renterd env and generate a seed

From the repo root:

```bash
cp renterd/.env.example renterd/.env
cd renterd
```

`RENTERD_SEED` and `RENTERD_API_PASSWORD` are required. Compose fails if
either is missing (same pattern as Headscale / Postgres).

Generate a 12-word seed (write it down; never commit it). Official wizard:

```bash
docker compose run --rm -it renterd config
```

When asked for a seed, type `seed` to generate one. Paste the phrase into
`renterd/.env` as `RENTERD_SEED=...`. Set `RENTERD_API_PASSWORD` to a long
random password. Pick an S3 access key (≥16 characters) and secret
(≥16 characters) and put them in `S3_ACCESS_KEY` / `S3_SECRET_KEY` in
`matchmaker/.env` (`RENTERD_S3_*` aliases also work). Optionally comment
the same pair in `renterd/.env` as a reminder. Never commit those files.

## 2. Start compose

```bash
cd renterd
docker compose up -d
```

Bindings (loopback on the operator host):

| Port | Service |
| --- | --- |
| 127.0.0.1:9980 | renterd HTTP API / UI (`--http :9980` inside the container) |
| 127.0.0.1:8081 | S3 gateway (container 8080; `RENTERD_S3_ADDRESS=0.0.0.0:8080`) |
| 127.0.0.1:9981 | Sia peer gateway |

UI: http://127.0.0.1:9980 (unlock with `RENTERD_API_PASSWORD`). First start
syncs the **zen** testnet (`RENTERD_NETWORK=zen`). Do not point CI or pytest
at this process.

## 3. Put S3 v4 keypairs into renterd (bus API)

After first boot, renterd has an S3 listener but empty v4 keypairs. PUT the
pair you chose in §1:

```bash
cd renterd
set -a && source .env && set +a
# ACCESS and SECRET are the Matchmaker S3_* (or RENTERD_S3_*) values, not the API password.
curl -sS -u ":${RENTERD_API_PASSWORD}" \
  -X PUT "http://127.0.0.1:9980/api/bus/settings/s3" \
  -H "Content-Type: application/json" \
  -d "{\"v4Keypairs\":{\"${S3_ACCESS_KEY:-$RENTERD_S3_ACCESS_KEY}\":\"${S3_SECRET_KEY:-$RENTERD_S3_SECRET_KEY}\"}}"
```

Confirm:

```bash
curl -sS -u ":${RENTERD_API_PASSWORD}" \
  "http://127.0.0.1:9980/api/bus/settings/s3"
```

You should see your access key in `v4Keypairs`. Do not paste the secret into
chat logs or git.

## 4. Point Matchmaker at the S3 gateway

In `matchmaker/.env` (never commit the real file):

```
S3_ENDPOINT=http://127.0.0.1:8081
S3_GUEST_ENDPOINT=
S3_REGION=us-east-1
S3_ACCESS_KEY=<same as renterd bus v4 keypair>
S3_SECRET_KEY=<same as renterd bus v4 keypair>
```

Leave `S3_GUEST_ENDPOINT` empty to reuse `S3_ENDPOINT`. `RENTERD_S3_*`
aliases work if you already named the vars that way. For
guests on other machines, set `S3_GUEST_ENDPOINT` to a mesh-reachable URL of this S3 port
(not the volunteer’s LAN IP). Restart uvicorn after editing.

Apply the storage migration if this Matchmaker database predates phase 15:

```bash
cd matchmaker
alembic upgrade head
```

## 5. Smoke: bucket grant (no live Sia upload required for the control plane)

With Matchmaker up and a logged-in user JWT:

```bash
curl -sS -X POST -H "Authorization: Bearer ${USER_JWT}" http://127.0.0.1:8000/v1/storage
curl -sS -H "Authorization: Bearer ${USER_JWT}" http://127.0.0.1:8000/v1/storage
```

Expect `provider: renterd`, `mountInsideGuest: true`, a `dcl-<uuid>` bucket,
an `objects` list (includes `files/.dcl/provisioned` after first issue), and
an `rclone` command that does **not** embed the secret. There is no
`accessKeyId` / `secretAccessKey` on the JSON. Booking `POST /v1/sessions`
returns the same locators under `storage`. `GET /v1/storage` is 404 until
issue or a book. If renterd S3 is down, that book returns 502 and does **not**
occupy a host.

## 6. Guest rclone (inside the VM only)

On the **guest**, not the volunteer host. Inject keys from operator env
(`S3_ACCESS_KEY` / `S3_SECRET_KEY` in Matchmaker `.env`) as
`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` — they are not returned by the
API. Prefer `storage.rcloneArgs` (argv) from `GET /v1/storage` or session
JSON over shell-splitting `storage.rclone`:

```bash
export AWS_ACCESS_KEY_ID=<S3_ACCESS_KEY from matchmaker/.env>
export AWS_SECRET_ACCESS_KEY=<S3_SECRET_KEY from matchmaker/.env>
rclone mount :s3:<bucket>/files /home/user/Persistent \
  --s3-provider Other \
  --s3-endpoint <endpoint> \
  --s3-env-auth \
  --s3-force-path-style \
  --s3-region us-east-1 \
  --s3-chunk-size 120MiB \
  --fast-list \
  --vfs-cache-mode full
```

Keep the VFS cache on the guest’s virtual disk. Do not point rclone at a
directory on the volunteer host. Host Agent `POST /v1/provision` may include
`storageEndpoint` (URL only) and must not mount.

Logout final flush and a “saved as of HH:MM” indicator still need a live
guest (GATE-FC / GATE-WHPX). This runbook does not close those gates.

## 7. How to report back

Record (no secrets):

1. `docker compose ps` shows `renterd` healthy.
2. `GET /api/bus/settings/s3` lists your access key (not the secret).
3. Matchmaker `POST` then `GET /v1/storage` returns 201/200 with `mountInsideGuest: true` and no access keys.
4. Confirm the Host Agent was **not** asked to rclone (provision body has
   `storageEndpoint` only).

If S3 create fails, Matchmaker returns 502 `renterd storage provision failed`
and does not leak keys.
