# Handoff — phase 13 signed base-image pipeline (2026-09-05)

- Phase goal: FR9 image slice — sign/verify guest image digest with a platform
  Ed25519 key; Host Agent refuses stub boot on mismatch. Software contract,
  not an image CDN.
- Verified git state at start: `30b45ef` on `main`, working tree clean, branch
  ahead of `origin/main` by 13. Did not push. GATE-FC / GATE-WHPX / GATE-PRO
  remain OPEN. Phase 12 in `PHASE_PLAN.md` is silence teardown, not signed
  images.
- Files created/changed:
  - `host-agent/src-tauri/src/image.rs`
  - `host-agent/src-tauri/src/provision_http.rs`, `provision.rs`, `lib.rs`
  - `host-agent/src-tauri/Cargo.toml`, `Cargo.lock`
  - `host-agent/.env.example`, `host-agent/README.md`
  - `matchmaker/src/matchmaker/images.py`
  - `matchmaker/src/matchmaker/routers/images.py`
  - `matchmaker/src/matchmaker/config.py`, `schemas.py`, `provision.py`,
    `deps.py`, `main.py`
  - `matchmaker/tests/test_images.py`, `tests/image_fixtures.py`,
    `tests/test_config.py`, `tests/test_provision.py`, `tests/conftest.py`
  - `matchmaker/.env.example`, `matchmaker/README.md`
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-13-signed-images.md` (this file)
- Key decisions made and why:
  - **Ed25519 over the raw 32-byte SHA-256 digest** (not the hex string).
    Standard base64 signature. Public key is 64 hex chars from env. Fail closed.
  - **Verify before `plan`.** OS-conditional stubs in `provision.rs` are
    unchanged. HTTP `POST /v1/provision` parses `imageDigest` +
    `imageSignature`, checks local standby digest, verifies the signature,
    then calls `plan`. Mismatch is 400; occupancy is not marked.
  - **No private key in git or runtime services.** Matchmaker advertises
    digest+sig from env. Agent pins the public key. Operator signs offline.
    `.env.example` holds local-dev fixture values derived from a test seed
    that exists only in Rust unit tests (`test-ed25519-image-key-seed-v001`).
    Not a production key. Never commit `.pem` / `.key` / `.env`.
  - **No multi-GB download.** Tests hash `dcl-fixture-standard-v1\n` /
    `dcl-fixture-pro-v1\n`. `IMAGE_LOCAL_DIGEST` stands in for a standby file.
  - **Additive Matchmaker schema.** Required `IMAGE_STANDARD_*` and
    `IMAGE_PRO_*`. `GET /v1/images/{labTier}` is public checksums. Session JSON
    unchanged. `HostSelector.select` and occupancy indexes unchanged.
  - **Listener fail-fast.** `HOST_AGENT_LISTEN_ADDR` still optional; if set,
    token + `IMAGE_SIGNING_PUBLIC_KEY` + `IMAGE_LOCAL_DIGEST` are required.
  - **Did not** change CSP, `freezePrototype`, GPU exclusivity, OS backends,
    Tailscale, Storj, or Stripe. Did not close GATE-*.
- Key env names:
  - Host Agent (when listening): `IMAGE_SIGNING_PUBLIC_KEY`,
    `IMAGE_LOCAL_DIGEST`
  - Matchmaker (startup): `IMAGE_STANDARD_DIGEST`,
    `IMAGE_STANDARD_SIGNATURE`, `IMAGE_PRO_DIGEST`, `IMAGE_PRO_SIGNATURE`
- Generate a test keypair (do not commit the PEM):

```bash
openssl genpkey -algorithm ED25519 -out platform-ed25519.pem
openssl pkey -in platform-ed25519.pem -pubout -outform DER | tail -c 32 | xxd -p -c 32
# IMAGE_SIGNING_PUBLIC_KEY

printf 'dcl-fixture-standard-v1\n' | shasum -a 256
# IMAGE_STANDARD_DIGEST / IMAGE_LOCAL_DIGEST

DIGEST_HEX=<64 hex chars>
echo "$DIGEST_HEX" | xxd -r -p > /tmp/image.digest
openssl pkeyutl -sign -inkey platform-ed25519.pem -rawin -in /tmp/image.digest | openssl base64 -A
# IMAGE_STANDARD_SIGNATURE
rm platform-ed25519.pem /tmp/image.digest
```

- Test results:
  - Matchmaker: **113 passed**, 0 failed. Was 105. Added image env fail-fast,
    GET `/v1/images/{labTier}`, provision payload digest+sig, source guard
    (no concrete selector / hypervisor names in image modules).
  - Host Agent: **109 passed**, 0 failed (`cargo test` in `src-tauri`). Was 93.
    Added valid signature accepts, tampered bytes reject, missing key/digest
    fail fast, HTTP mismatch refuses stub boot / occupancy.
  - Web: **27 passed**, 0 failed. `npm run typecheck` clean.
  - `cargo clippy -- -D warnings` clean on the lib.
- What’s built / verified / needs hardware / assumptions:
  - **Built:** signed-image software contract + registry metadata.
  - **Verified:** pytest/cargo/vitest counts above.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO unchanged. A real guest
    image is still not in-repo.
  - **Assumptions:** local `.env` copies fixture `IMAGE_*` values; existing
    Matchmaker processes must be restarted.
- Flagged for the next agent:
  - **GATE-FC / GATE-WHPX / GATE-PRO remain OPEN.** No `gate-*-result.md`.
  - Next software phase is **14** (Tailscale/Headscale). **Needs a human
    account — do not implement unattended.**
  - Restart Matchmaker with `IMAGE_*` set. Do not commit `.env`.
  - Aikido MCP `namespaceStatus: error`; not a scan pass. Run `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; working tree should
  be clean; still ahead of `origin/main`; did not push.
