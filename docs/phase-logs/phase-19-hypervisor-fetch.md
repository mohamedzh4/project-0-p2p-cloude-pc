# Handoff — phase 19 hypervisor binary fetch (2026-09-06)

- Phase goal: After the capability probe, locate or fetch the OS hypervisor
  binary (Linux Firecracker; Windows QEMU stub) and log path/version without
  booting a guest.
- Verified git state at start: `3eb7862` on local `main`, working tree clean,
  ahead of `origin/main`. Did not invent hardware logs. Did not create
  `gate-*-result.md`. `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.
  Did not push.
- Files created/changed:
  - `host-agent/src-tauri/src/hypervisor.rs` (new; no Tauri; never invokes)
  - `host-agent/src-tauri/src/lib.rs`, `commands.rs`, `Cargo.toml`, `Cargo.lock`
  - `host-agent/src/types.ts`, `api.ts`, `preview.ts`, `App.tsx`
  - `host-agent/src/components/CapabilityPanel.tsx`
  - `host-agent/README.md`, `host-agent/.env.example`
  - `PHASE_PLAN.md` (phase 19 done + 19b leftover)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-19-hypervisor-fetch.md` (this file)
- Key decisions made and why:
  - **Linux Firecracker fetch is real; Windows QEMU is 19b.** Silent Windows
    QEMU install is too heavy for this session. Windows Standard with the
    probe offering labs logs `would fetch QEMU` from the documented
    weilnetz index and does not download. Flag **19b**.
  - **macOS skips.** Does not pretend to fetch Firecracker. Log includes the
    probe reason. Linux/Windows with Standard not offered fail fast (no
    download).
  - **Checksum fail-fast.** SHA-256 of the artifact must match the documented
    GitHub `.sha256.txt` (Firecracker v1.16.1) or env override. Mismatch does
    not install. Cache is user-writable (`HOST_AGENT_HYPERVISOR_CACHE` or
    platform user cache), not `/usr/bin`. Binary is never executed.
  - **Tests never hit the internet by default.** Injected fetcher, `file://`,
    or loopback `tiny_http`. `HttpBinaryFetcher` refuses non-loopback HTTP
    under `cfg(test)` unless `HOST_AGENT_HYPERVISOR_NETWORK=1`.
  - **CSP/webview still does not fetch.** Status is a Tauri command. OS split
    is a peer `match` on `HostOs` like `provision.rs`. `HostSelector.select`,
    occupancy indexes, GPU exclusivity copy unchanged.
- Test results:
  - Host Agent: **159 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 143. Added macOS skip, Linux fetch/present/checksum, Windows
    would-fetch, peer paths, file://, httptest, documented URL, env override.
  - Matchmaker: **161 passed**, 0 failed. Unchanged.
  - Web: **31 passed**, 0 failed. Host Agent `npm run typecheck` passed.
  - Host Agent preview (`localhost:1420`): hypervisor card shows sample
    Firecracker already present; Standard/Pro offered; GPU exclusivity copy
    still present. Packaged agent on this Mac logs skip (unsupported OS).
- What’s built / verified / needs hardware / still paper:
  - **Built:** Firecracker locate/fetch + log; Windows QEMU stub log; UI line.
  - **Verified:** test counts above. No GitHub download in CI tests.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO remain **OPEN**.
  - **Still paper:** Windows QEMU silent install (**19b**); guest image fetch
    (phase 20); launch config; VFIO bind; pairwise ACLs; live guest
    Tailscale/rclone; FR13 `latency_ms`.
- Flagged for the next agent:
  - Start at **phase 20** (guest image fetch/verify). Do not invoke
    Firecracker/QEMU. Do not mark gates closed. Do not create
    `gate-*-result.md`. **19b** is Windows QEMU silent install if that is
    scheduled before metal.
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; still ahead of
  `origin/main`; did not push.
