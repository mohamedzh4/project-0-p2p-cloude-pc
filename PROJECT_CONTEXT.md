# Distributed Compute Lab — project context

## Overview

A peer-to-peer compute platform: people with idle high-performance PCs ("Hosts")
rent out CPU/RAM/GPU to users who need more power, delivered as an on-demand
virtual desktop in a browser rather than from a centralized datacenter.

Four planes: a custom **Matchmaker** (auth, matching, billing, session
lifecycle), a **compute** plane on host PCs the owner still uses (Standard Lab:
Firecracker on Linux, QEMU+WHPX on Windows; Pro Lab: exclusive GPU assignment
via VFIO or Hyper-V), a **network** plane (Tailscale/Headscale), and a
**storage** plane (Storj via rclone).

**Written so far:** two spec documents and one module — the Host Agent, in Rust +
Tauri v2 + React/TypeScript (~800 lines Rust, ~460 lines UI).

## Where we actually are

Phases below come from `docs/cahier-des-charges-distributed-compute-lab.md` §11.

| Phase | Content | Status |
| --- | --- | --- |
| 0 | Manual proof of the compute pipeline — Standard on Linux (Firecracker+Kasm) and on Windows (QEMU+WHPX+Kasm), plus one Pro GPU session, by hand | **Not started — this is the real next step** |
| 1 | MVP Standard Lab, single region, hosts approved manually | Not started |
| 2 | Full Matchmaker (matching + billing), automated host onboarding | Host Agent begun: hardware detection + resource caps only |
| 3 | Pro/Gamer Lab, latency-aware matching | Not started |
| 4 | Hardening: image signing, exit nodes, reliability scoring | Not started |

Against the host-side requirements: **FR8** (declare a concurrent CPU/RAM slice,
and optionally which GPU to dedicate — exclusive for the session) is done except
the availability schedule. **FR9** (auto-join mesh, pull signed base image),
**FR10** (idle/in-session/earnings view) and **FR11** (pause sharing) are
untouched.

The spec deliberately orders work by risk, not by layer: the Matchmaker is
conventional backend work, while the compute pipeline feeling native is the
actual technical bet. That bet is still unvalidated.

## Layout

```
project-0-p2p-cloude-pc/
├── docs/
│   ├── distributed-compute-lab-architecture.md
│   └── cahier-des-charges-distributed-compute-lab.md
└── host-agent/
    ├── src/          React UI — App, api, components, format, preview, types
    └── src-tauri/    Rust — models, hardware, gpu, config, commands, lib
```

Remote: [github.com/mohamedzh4/project-0-p2p-cloude-pc](https://github.com/mohamedzh4/project-0-p2p-cloude-pc)

## Architecture decisions still true today

| Decision | Status |
| --- | --- |
| Integration over invention — only the Matchmaker is genuinely custom | Adopted (architecture §1) |
| Two-tier split at the GPU boundary — Firecracker ships no PCI passthrough, so Pro needs QEMU+VFIO / Hyper-V GPU assignment | Forced by Firecracker's design, not a preference |
| Standard Lab backend is OS-conditional: Firecracker on Linux, QEMU+WHPX (or Hyper-V) on Windows. Firecracker does not run on Windows | Adopted |
| Hosts keep using the PC. CPU/RAM sharing is a concurrent slice; GPU sharing on consumer hardware is exclusive for the session | Adopted |
| Kasm runs *inside* the guest VM; the hypervisor (Firecracker or QEMU/Hyper-V) is the isolation boundary | Adopted on paper |
| Hosts are stateless and zero-trust; nothing survives a session | Adopted, not yet implemented |
| Guest egress routes through platform-run exit nodes, never the host's home IP | Adopted on paper — real legal exposure if skipped |
| Sync on a 5-10 min timer *plus* event-triggered flushes on save and logout | Adopted on paper |
| Host Agent is Rust + Tauri v2, not Electron | Implemented |
| Rust core takes plain paths and structs, no Tauri types in detection or storage | Implemented, so it can become a headless service |
| Webview is untrusted — every config write is re-clamped in Rust | Implemented |
| Windows + Linux are the real host targets; macOS is dev-only | Implemented (a Mac host would need QEMU/Tart as a second VM path) |
| Matchmaker API, database, auth, user web app | Not started, not decided |

## Active rules

Rules live at the workspace root in `../.cursor/`, and apply here.

**Security** (`security.mdc`, always on)
- Secrets from env vars only; fail fast when missing, never fall back to a literal.
- Never commit `.env`, `service-account.json`, or `*.pem`; only `.env.example` placeholders.
- Validate input before the DB layer; parameterized queries, never string concatenation.

**Design & animation** (`design.mdc`, auto-attached to `*.tsx`/`*.jsx`/`*.css`)
- A named 4-6 color palette; no Tailwind defaults shipped as the final look.
- No generic AI aesthetics (cream+serif+terracotta, near-black+neon, newspaper hairlines).
- One display font paired with one body font.
- One orchestrated motion moment, using `transform`/`opacity` only.
- Visible `:focus-visible` everywhere; honor `prefers-reduced-motion`.
- Playwright screenshot after any UI change.

**Enforcement** — `hooks.json` + `protect.py` fail-closed guard on shell and edits;
`security-reviewer` agent reports findings read-only; `/security-audit` and
`/sync-context` commands.

## Current state of the Host Agent

**Working**

- CPU (model, logical + physical cores) and RAM via `sysinfo` 0.39, with `default-features = false, features = ["system"]` and the narrowest refresh available — no process list, disks, or networks.
- GPU via `nvml-wrapper` 0.12, gated at the Cargo level to Windows/Linux. Every failure path returns an empty list plus a note saying which case it was, so the UI can show "No GPU detected" *and* the reason.
- Window: hardware cards, sharing toggle, CPU/RAM sliders (concurrent slice), GPU picker with exclusivity warning when a card is present.
- `sharing-config.json` in the OS app-config dir, written temp-file-then-rename, re-clamped to the current machine on every read, rejected above 64 KiB. v2 records `sharedGpuId`; v1 files still load.
- `--print-hardware` prints detection as JSON with no window.
- Icon generated from code (`scripts/generate-icon.mjs`), not a committed blob.

**Verified:** 30 Rust tests pass, clippy clean at `-D warnings`, rustfmt clean,
frontend typechecks and builds, release binary 2.9 MB. Ran on an Apple M1 and
correctly reported 8 cores / 8 GiB / no GPU.

**Not built:** networking, mesh join, Matchmaker registration, heartbeats, base
images, VM provisioning, session lifecycle, earnings, schedule, tray/background
mode, pause-sharing, and the entire user-facing web app.

**Known issues**

- The Aikido MCP scanner still fails to start in this Cursor workspace (plugin in `error` / no response). Local `security-reviewer` is the fallback until `/aikido:setup` works after Cursor reloads the plugin.
- This Mac cannot be a Standard or Pro *host*: Firecracker is Linux/KVM only, Windows Standard Lab needs QEMU+WHPX. Phase 0 needs a Linux box and a Windows box with virtualization enabled (plus a spare GPU or iGPU for the Pro tier). macOS remains a development machine for the agent UI and logic.

## Recent changes

See `git log --oneline -10` on `main`. Latest work (not yet summarized in older commits): OS-conditional Standard Lab (Firecracker on Linux, QEMU+WHPX on Windows), GPU sharing documented and UI'd as exclusive for the session, `sharedGpuId` in config v2, `freezePrototype` on, and capabilities narrowed off `core:default`.
