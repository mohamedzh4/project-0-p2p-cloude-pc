# Handoff — phase 20 guest image fetch/verify (2026-09-06)

- Phase goal: Fetch the signed base image for the probed backend into a user
  cache and verify SHA-256 + Ed25519 before it is eligible for launch. Do not
  boot. Extend `image.rs`; do not replace it.
- Verified git state at start: `0c62d0e` on local `main`, working tree clean,
  ahead of `origin/main`. Did not invent hardware logs. Did not create
  `gate-*-result.md`. `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.
  Did not push. Did not start phase 21.
- Files created/changed:
  - `host-agent/src-tauri/src/image.rs` (extended; existing verify kept)
  - `host-agent/src-tauri/src/lib.rs`, `commands.rs`
  - `host-agent/src/types.ts`, `api.ts`, `preview.ts`, `App.tsx`
  - `host-agent/src/components/CapabilityPanel.tsx`
  - `host-agent/README.md`, `host-agent/.env.example`
  - `PHASE_PLAN.md` (phase 20 done)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-20-guest-image-fetch.md` (this file)
- Key decisions made and why:
  - **Linux Firecracker fetch is real; Windows is would-fetch.** Linux Standard
    fetches the signed guest image (Matchmaker `IMAGE_STANDARD_*` digest+sig +
    `HOST_AGENT_STANDARD_IMAGE_URL`). Linux Pro, when offered, fetches the Pro
    image the same way. Windows Standard logs would-fetch a QEMU disk and does
    not download. macOS skips with a logged reason — it does not pretend
    Firecracker images run here.
  - **Verify before ready.** SHA-256 of the bytes must match the advertised
    digest, then Ed25519 over the raw 32-byte digest with
    `IMAGE_SIGNING_PUBLIC_KEY`. Mismatch deletes the temp file and does not
    leave a ready disk. `IMAGE_LOCAL_DIGEST` pins Standard; a registry digest
    that differs fails fast. Existing `verify_image` (provision path) is
    unchanged.
  - **Reuse Matchmaker `IMAGE_*` metadata.** Digest+signature env names match
    `GET /v1/images/{labTier}`. Matchmaker still does not host blobs and still
    holds no private key. Bytes URL is operator-supplied on the agent.
  - **Tests never download a real disk.** Injected fetcher, `file://`, or
    loopback `tiny_http`. Non-loopback HTTP is refused under `cfg(test)` unless
    `HOST_AGENT_IMAGE_NETWORK=1`. Fixture bytes remain
    `dcl-fixture-standard-v1\n` / `dcl-fixture-pro-v1\n`.
  - **CSP/webview still does not fetch.** Status is a Tauri command. OS split
    is a peer `match` on `HostOs` like `hypervisor.rs` / `provision.rs`.
    `HostSelector.select`, occupancy indexes, GPU exclusivity copy unchanged.
- Test results:
  - Host Agent: **176 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 159. Added macOS skip, Linux fetch/present/reject, Pro dual-fetch,
    Windows would-fetch, peer paths, file://, httptest, env metadata reuse,
    local-digest pin mismatch. Existing fixture verify tests stayed green.
  - Matchmaker: **161 passed**, 0 failed. Unchanged (no image-router change;
    still checksums only, not a blob download).
  - Web: **31 passed**, 0 failed. Host Agent `npm run typecheck` passed.
- What’s built / verified / needs hardware / still paper:
  - **Built:** guest-image locate/fetch + SHA-256/Ed25519 before ready; UI line.
  - **Verified:** test counts above. No multi-GB download in CI tests.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO remain **OPEN**.
  - **Still paper:** production guest disk (not in-repo); Windows QEMU silent
    install (**19b**); launch config (phase 21); VFIO bind; pairwise ACLs;
    live guest Tailscale/rclone; FR13 `latency_ms`.
- Flagged for the next agent:
  - Start at **phase 21** (launch config from sliders + GPU id). Do not invoke
    Firecracker/QEMU. Do not mark gates closed. Do not create
    `gate-*-result.md`. Do not boot a VM.
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; still ahead of
  `origin/main`; did not push.
