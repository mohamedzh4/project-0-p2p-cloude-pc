# Handoff — phase 33 Host Agent copyable binary (2026-09-07)

- Phase goal: Operator can build Host Agent **on Linux or Windows**, copy the
  artifact, set env, leave it running (detect → register → heartbeat →
  provision boot/apply). No fake MSI/AppImage in git. This Mac cannot host
  or emit those installers.
- Files created/changed:
  - `host-agent/src-tauri/src/env_file.rs` (packaged `.env` paths + tests)
  - `host-agent/src-tauri/src/lib.rs` (load those paths; Hyper-V VM from env)
  - `host-agent/src-tauri/src/matchmaker.rs` (fail-fast if `agentUrl` would be
    `0.0.0.0` / `[::]`; tests)
  - `host-agent/src-tauri/src/gpu_bind.rs` (`HOST_AGENT_HYPERV_VM`)
  - `host-agent/src-tauri/capabilities/default.json` (shell plugin still absent)
  - `host-agent/.env.example`, `host-agent/README.md`
  - `.github/workflows/host-agent.yml` (ubuntu-22.04 + windows-latest;
    `npm run tauri build`; upload artifacts; no release secrets)
  - `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`,
    `docs/distributed-compute-lab-architecture.md`, `guest-images/README.md`,
    `matchmaker/README.md`
  - `docs/phase-logs/phase-33-host-agent-binary.md` (this file)
- Key decisions made and why:
  - **On-target build only.** Linux:
    `npm run tauri build -- --bundles appimage,deb`. Windows:
    `npm run tauri build -- --bundles msi,nsis`. Artifacts under
    `src-tauri/target/release/bundle/` plus the unsigned binary. Not committed.
  - Packaged apps load `.env` from cwd, parent, exe dir, app-config dir, or
    `HOST_AGENT_ENV_FILE` (must exist if set). Process env still wins.
  - `HOST_AGENT_LISTEN_ADDR=0.0.0.0:7842` in `.env.example` so a volunteer
    actually listens. Derived `http://0.0.0.0:7842` is **not** a Matchmaker
    destination — register fails until `HOST_AGENT_URL` is a reachable origin.
  - No Tauri shell/sidecar permissions. Firecracker/QEMU/PowerShell/tailscale
    stay `std::process::Command` in Rust. CSP unchanged.
  - Guest images remain Packer (`guest-images/scripts/build.sh`); not hidden.
  - `GET /v1/sessions` 405 on a five-day-old uvicorn is **restart Matchmaker**,
    not a harness bug (`POST` books, `GET` is history).
  - Optional CI workflow; does not run until pushed. No cross-compile from
    macOS.
- Test results: **328/328** `cargo test --lib` (was 320; +8: env file paths,
  unspecified `agentUrl` bind, Hyper-V VM env). Pytest 166 / vitest 52 not
  re-run (untouched). Aikido: MCP `namespaceStatus: error`; `mcp_auth` already
  handled / tool discovery failed. Not a scan pass. Run `/aikido:setup`.
- Flagged for the orchestrator:
  - Hardware lines stay **Implemented in code. Not yet run on physical
    hardware.** TAP/net, Packer disk on metal, `FIRECRACKER_KERNEL`, Windows
    UAC, Hyper-V `dcl-pro-guest`, and a human live session are still required.
  - Numbered software phases 1–33 are complete at software level except that
    metal confirmation. Do not invent a live desktop.
  - `origin/main` still behind. Nothing pushed. Do not touch `pr/*`.
