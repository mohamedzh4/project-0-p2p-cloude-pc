# Handoff — phase 24 Windows QEMU silent install (2026-09-06)

- Phase goal: Detect missing QEMU on Windows and run the official Weilnetz
  NSIS installer with unattended flags, without booting a guest and without
  hitting the QEMU CDN from tests.
- Verified git state at start: `db6d994` on local `main`, working tree clean,
  ahead of `origin/main`. Did not invent hardware logs. Did not create
  `gate-*-result.md`. `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.
  Did not push. Did not start leftover **22b**. Did not close gates.
- Files created/changed:
  - `host-agent/src-tauri/src/hypervisor.rs` (Windows NSIS install path)
  - `host-agent/src-tauri/src/lib.rs` (inject `ProcessQemuInstaller`)
  - `host-agent/src/types.ts`, `host-agent/src/components/CapabilityPanel.tsx`
  - `host-agent/README.md`, `host-agent/.env.example`
  - `docs/runbooks/gate-whpx.md` (silent install + UAC), `gate-fc.md` /
    `gate-pro.md` (automation pointer 18–24)
  - `PHASE_PLAN.md` (phase 24 done; **19b** closed; **22b** leftover)
  - `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
  - `docs/distributed-compute-lab-architecture.md`
  - `docs/phase-logs/phase-24-windows-qemu-silent-install.md` (this file)
- Key decisions made and why:
  - **Official NSIS flags, not guessed.** Weilnetz installers are built from
    QEMU `qemu.nsi`. NSIS Chapter 3 documents `/S` (silent, case-sensitive)
    and `/D=dir` (install directory; last argv; unquoted). Pinned artifacts:
    `qemu-w64-setup-20260811.exe` and `qemu-arm-setup-20260811.exe` with the
    published `.sha512` sidecars on qemu.weilnetz.de.
  - **Injectable installer, same as Firecracker fetch.** CI passes a fake
    `QemuInstaller` plus a fixture payload / `file://` / loopback httptest.
    Production uses `ProcessQemuInstaller`. Tests never download 197 MiB.
  - **UAC is the only remaining manual step.** `qemu.nsi` has
    `RequestExecutionLevel admin`. `/S` does not suppress Windows UAC.
    Approving or denying that prompt is not automated. Non-zero or missing
    `qemu-system-*.exe` is `InstallerFailed` — never a false “installed”.
  - **Do not execute QEMU.** Verify the install path exists. Never spawn
    `qemu-system-*.exe`. GATE-WHPX stays OPEN. Leftover **22b** untouched.
    `HostSelector.select`, occupancy indexes, CSP, Argon2id, JWT unchanged.
- Test results:
  - Host Agent: **238 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
    Was 231. Added silent argv `/S` then `/D=`, UAC-cannot-suppress contract,
    installer non-zero / exit-0-without-binary not installed, checksum skip
    of installer, skip-when-present, documented Weilnetz URL+SHA-512, env
    override. Peer Linux Firecracker / Windows QEMU both `Fetched` in tests.
  - Matchmaker: **161 passed**, 0 failed. Unchanged (not run this session;
    no Matchmaker files touched).
  - Web: **31 passed**, 0 failed. Unchanged (not run this session; no web
    files touched). Host Agent `npm run typecheck` passed.
- What’s built / verified / needs hardware / still paper:
  - **Built:** Windows QEMU detect → fetch → SHA-512 → NSIS `/S` `/D=` →
    verify `qemu-system-*.exe` in the user cache.
  - **Verified:** test counts above. This Mac cannot run the real NSIS
    installer; tests inject it.
  - **Needs hardware:** GATE-FC / GATE-WHPX / GATE-PRO remain **OPEN**.
  - **Still paper:** live QEMU+WHPX boot; Windows DDA apply (**22b**);
    pairwise ACLs; live guest Tailscale/rclone; FR13 `latency_ms`. Guest
    image on Windows is still would-fetch (phase 20).
- Flagged for the next agent:
  - Leftover **22b** is live Hyper-V DDA/GPU-P apply (do not spawn
    PowerShell). Do not invoke Firecracker/QEMU. Do not mark gates closed.
    Do not create `gate-*-result.md`.
  - **Minimal unavoidable manual step:** on a real Windows host, approve
    (or deny) the UAC prompt when the NSIS installer elevates.
  - Aikido MCP `namespaceStatus: error` this session; not a scan pass. Run
    `/aikido:setup`.
  - `origin/main` still behind. Nothing pushed.
- Verified git state at end: this commit on local `main`; still ahead of
  `origin/main`; did not push.
