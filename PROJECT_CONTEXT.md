# Distributed Compute Lab — project context

Living snapshot: [`PROJECT_STATUS.md`](PROJECT_STATUS.md).
Agent execution plan: [`PHASE_PLAN.md`](PHASE_PLAN.md).
Handoffs: [`docs/phase-logs/`](docs/phase-logs/). Prefer those over this file
for “where we are.”

## Overview

A peer-to-peer compute platform: people with idle high-performance PCs ("Hosts")
share a CPU/RAM slice (and optionally one exclusive GPU) while they keep using
the machine. Guests get an on-demand virtual desktop in a browser. Stack so far:
Host Agent (Rust + Tauri v2 + React), Matchmaker (Python FastAPI + Postgres),
and a thin React harness in `web/`. Manual Firecracker/QEMU+Kasm validation is
**not evidenced in this repo** (open hardware gate). No Tailscale or Storj.
Matchmaker-triggered provision is a **stub** (fake/HTTP port, no hypervisor).

Remote: [github.com/mohamedzh4/project-0-p2p-cloude-pc](https://github.com/mohamedzh4/project-0-p2p-cloude-pc)

## Where we actually are

From `docs/cahier-des-charges-distributed-compute-lab.md` §11.

| Phase | Content | Status |
| --- | --- | --- |
| 0 | Manual Firecracker+Kasm (Linux) and QEMU+WHPX+Kasm (Windows), plus one Pro GPU session | **Open / not evidenced in-repo** — runbooks in `docs/runbooks/`; no human result file, guest image, or live Kasm URL. This Mac cannot host. |
| 1 | MVP Standard Lab, hosts approved manually | **Software loop done (execution phases 1–13):** Matchmaker auth + host inventory (pending → manual approve) + occupancy + heartbeats + internal credits ledger + silence teardown/`interrupted` + rematch via `HostSelector`; Host Agent Rust client; OS-conditional **provisioner stubs** + Matchmaker `HostProvisioner`; signed image digest+Ed25519 verify before stub boot; `web/` harness with FR2 intent → `labTier` and session history; `RankedHostSelector` on declared caps; Host Agent tray + immediate pause + idle/in-session/paused + accrued earnings. **Not a usable desktop** until GATE-FC / GATE-WHPX. Next software: execution phase 14 (mesh — ask human first). |
| 2 | Full Matchmaker (matching + billing), automated host onboarding | Matching is `RankedHostSelector` (declared caps; no latency/reliability telemetry). Internal ledger meters sessions; payouts deferred. Silence teardown + rematch in. Plan phases 13–15. |
| 3 | Pro/Gamer Lab, latency-aware matching | Stubs only (Pro backends in `provision.rs`). Phase 16 + GATE-PRO. |
| 4 | Hardening: image signing, exit nodes, reliability scoring | Image signing software contract in (phase 13). Exit nodes: phase 14. |

**FR1** signup/login: done. **FR2** intent → `labTier`: harness mapping in `web/src/intent.ts`; Pro 409 if no GPU host. **FR3** most suitable host: `RankedHostSelector` (declared CPU then RAM; Pro dedicated GPU id). **FR8** CPU/RAM + optional dedicated GPU: done except schedule. **FR5** streamed desktop: harness can open a URL; seed is still `example.invalid`. **FR7** session history + internal credits. **FR10** idle / in-session / paused + accrued earnings (not paid out); Rust tray. **FR11** immediate pause stops heartbeats (after 45s Matchmaker treats that as silence). **FR12** inventory: caps + heartbeats; ranking uses declared caps; no load/reliability score. **FR13** partial (tier + caps; no latency). **FR14**: mock provision + heartbeats + occupied silence teardown; no live hypervisor. **FR16** metering done; payouts deferred. **FR15** software rematch on silence (no Storj follow-along). **FR9** image verify in (no mesh). **FR4, FR6** not implemented (see `PHASE_PLAN.md` phases 14–16).

## Layout

```
project-0-p2p-cloude-pc/
├── docs/           architecture, cahier, runbooks, phase-logs
├── host-agent/     Tauri tray: hardware probe, sharing UI, pause, local config
├── matchmaker/     FastAPI + Alembic + Postgres
└── web/            React harness against the Matchmaker API
```

## Architecture decisions still true today

| Decision | Status |
| --- | --- |
| Only the Matchmaker is custom glue; everything else is reused | Adopted |
| Standard Lab backend is OS-conditional: Firecracker on Linux, QEMU+WHPX on Windows | Adopted in **stubs**; hardware validation not recorded (runbooks exist, gates open) |
| Pro needs VFIO / Hyper-V GPU assignment — Firecracker has no PCI passthrough | Forced; Pro stubs exist, no live passthrough |
| Hosts keep using the PC. CPU/RAM is a concurrent slice; GPU is exclusive for the session | Adopted in Host Agent UI + config v2 |
| Kasm runs *inside* the guest; the hypervisor is the isolation boundary | Adopted on paper; not evidenced in-repo |
| Guest egress via platform exit nodes, never the host's home IP | On paper (phase 14 — ask human first) |
| Host Agent is Rust + Tauri v2; detection/config take plain paths, no Tauri types | Implemented |
| Webview is untrusted — config writes are re-clamped in Rust | Implemented |
| Matchmaker is a separate FastAPI+SQLAlchemy+Alembic+Postgres service | Implemented |
| Argon2id passwords, JWT HS256, secrets from env, fail fast if missing | Implemented |
| Occupancy from active sessions (partial unique indexes), not a host flag | Implemented |
| Matching behind `HostSelector` so a later algorithm does not rewrite the session router | Implemented (`RankedHostSelector`) |
| `web/` is a Matchmaker test harness, not the product dashboard | Implemented |

## Active rules

Live in workspace `../.cursor/`.

**Security** (`security.mdc`) — secrets from env only; never commit `.env` / keys / PEM; parameterized queries / ORM only.

**Design** (`design.mdc`, `*.tsx`/`*.jsx`/`*.css`) — named 4–6 color palette; one display + one body font; one motion beat with `transform`/`opacity`; `:focus-visible`; `prefers-reduced-motion`; Playwright screenshot after UI changes.

**Git** (`git-commits.mdc`) — no AI attribution (`Co-authored-by`, "Generated with") in commits or PRs; imperative subject.

**Enforcement** — `hooks.json` + `protect.py`; `security-reviewer` (read-only); `/security-audit`, `/sync-context`.

## Current state

**Host Agent (`host-agent/`)** — CPU/RAM via `sysinfo` 0.39; NVIDIA GPUs via `nvml-wrapper` (Windows/Linux), graceful empty list otherwise. UI: sliders + GPU picker with exclusivity copy, status board (paused / idle / in-session), immediate pause, accrued earnings from Matchmaker. Config v2 (`sharedGpuId`); v1 files still load. Rust Matchmaker client registers clamped caps, pings every 10s while sharing is on, and GETs host earnings. Provisioner stubs: `plan_provision` (Linux→Firecracker, Windows→QEMU+WHPX, Pro→VFIO/Hyper-V GPU, macOS unsupported) plus opt-in `POST /v1/provision` that verifies SHA-256 + Ed25519 **before** the stub plan. Rust tray (no JS tray APIs). Image public key from env when listening. **109** Rust tests; clippy `-D warnings`. Not built: mesh join, real VMs, payouts.

**Matchmaker (`matchmaker/`)** — signup/login/me; host register/update (pending) + manual approve; heartbeats stamp `lastSeenAt`; `RankedHostSelector` ranks declared CPU/RAM among approved live free hosts (Pro needs a dedicated GPU id); `POST /v1/sessions` calls a `HostProvisioner` after occupancy commit (body includes image digest+signature); silent occupied hosts are `interrupted` and rematched via `HostSelector.select`; session end writes `ledger_entries`; `GET /v1/images/{labTier}` advertises checksums; camelCase JSON; Alembic through `0004_ledger_entries`. **113 pytest cases**. `SEED_HOST_KASM_URL` is still the `example.invalid` placeholder. Needs `HOST_BOOTSTRAP_TOKEN`, `HOST_APPROVE_TOKEN`, `CREDITS_PER_MINUTE`, and `IMAGE_*` digest/signature pairs in `.env` to boot.

**Web (`web/`)** — Vite/React harness: signup/login, FR2 intent prompt mapped to `labTier`, request session, open a **live** Kasm URL in a new tab (not `.invalid`), end session, session history + credits. Needs `VITE_MATCHMAKER_URL`. Vitest for intent mapping, placeholders, 200/409 mapping, API errors, history GET. Not a payment UI, not an iframe.

**Known issues**

- Matchmaker / `web/` / Host Agent are on local `main` after execution phase 13; `origin/main` is still behind until someone pushes.
- Aikido MCP still fails here; use `security-reviewer` until `/aikido:setup` works.
- This Mac cannot *host*: Firecracker is Linux/KVM; Windows Standard needs QEMU+WHPX. Follow `docs/runbooks/` — do not mark GATE-* closed without a human result.

## Recent changes

Execution phase 13 added signed base-image verify (Ed25519 over SHA-256)
before the Host Agent stub boot, plus Matchmaker image registry metadata.
No image CDN and no hypervisor. Hardware gates remain open.
See `PROJECT_STATUS.md`.
