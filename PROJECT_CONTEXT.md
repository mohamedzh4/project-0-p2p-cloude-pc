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
**not evidenced in this repo** (open hardware gate). No Tailscale, Storj, or
Matchmaker-triggered provision yet.

Remote: [github.com/mohamedzh4/project-0-p2p-cloude-pc](https://github.com/mohamedzh4/project-0-p2p-cloude-pc)

## Where we actually are

From `docs/cahier-des-charges-distributed-compute-lab.md` §11.

| Phase | Content | Status |
| --- | --- | --- |
| 0 | Manual Firecracker+Kasm (Linux) and QEMU+WHPX+Kasm (Windows), plus one Pro GPU session | **Open / not evidenced in-repo** — runbooks in `docs/runbooks/`; no human result file, guest image, or live Kasm URL. This Mac cannot host. |
| 1 | MVP Standard Lab, hosts approved manually | **In progress (software):** Matchmaker auth + host inventory (pending → manual approve) + occupancy; Host Agent Rust client registers clamped v2 caps; `web/` signup/request/open-desktop/end, on local `main` (not pushed). Next software: execution phase 4 (heartbeats). |
| 2 | Full Matchmaker (matching + billing), automated host onboarding | Host Agent: detection + caps only. Matching is `SingleApprovedHostSelector`, not a real algorithm |
| 3 | Pro/Gamer Lab, latency-aware matching | Not started |
| 4 | Hardening: image signing, exit nodes, reliability scoring | Not started |

**FR1** (signup/login) done in Matchmaker + `web/`. **FR8** (CPU/RAM slice + optional dedicated GPU) done in the Host Agent except schedule. **FR5** (streamed desktop) needs a live `SEED_HOST_KASM_URL`; the seed example is still `example.invalid`. The harness opens that URL; it does not embed Kasm. **FR12** has register/update of declared caps from both Matchmaker API and the Host Agent Rust client; hosts start pending and are approved manually. No heartbeat/load/reliability yet. **FR3/FR14** still have occupancy + one bookable approved host, no VM boot from the Matchmaker. **FR2, FR4, FR6–FR7, FR9–FR11, FR16** untouched.

## Layout

```
project-0-p2p-cloude-pc/
├── docs/           architecture, cahier, runbooks, phase-logs
├── host-agent/     Tauri tray-to-be: hardware probe, sharing UI, local config
├── matchmaker/     FastAPI + Alembic + Postgres
└── web/            React harness against the Matchmaker API
```

## Architecture decisions still true today

| Decision | Status |
| --- | --- |
| Only the Matchmaker is custom glue; everything else is reused | Adopted |
| Standard Lab backend is OS-conditional: Firecracker on Linux, QEMU+WHPX on Windows | Adopted on paper; hardware validation not recorded (runbooks exist, gates open) |
| Pro needs VFIO / Hyper-V GPU assignment — Firecracker has no PCI passthrough | Forced |
| Hosts keep using the PC. CPU/RAM is a concurrent slice; GPU is exclusive for the session | Adopted |
| Kasm runs *inside* the guest; the hypervisor is the isolation boundary | Adopted on paper; not evidenced in-repo |
| Guest egress via platform exit nodes, never the host's home IP | On paper |
| Host Agent is Rust + Tauri v2; detection/config take plain paths, no Tauri types | Implemented |
| Webview is untrusted — config writes are re-clamped in Rust | Implemented |
| Matchmaker is a separate FastAPI+SQLAlchemy+Alembic+Postgres service | Implemented (Phase 1 slice) |
| Argon2id passwords, JWT HS256, secrets from env, fail fast if missing | Implemented |
| Occupancy from active sessions (partial unique indexes), not a host flag | Implemented |
| Matching behind `HostSelector` so a later algorithm does not rewrite the session router | Implemented (`SingleApprovedHostSelector`) |
| `web/` is a Matchmaker test harness, not the product dashboard | Implemented |

## Active rules

Live in workspace `../.cursor/`.

**Security** (`security.mdc`) — secrets from env only; never commit `.env` / keys / PEM; parameterized queries / ORM only.

**Design** (`design.mdc`, `*.tsx`/`*.jsx`/`*.css`) — named 4–6 color palette; one display + one body font; one motion beat with `transform`/`opacity`; `:focus-visible`; `prefers-reduced-motion`; Playwright screenshot after UI changes.

**Git** (`git-commits.mdc`) — no AI attribution (`Co-authored-by`, "Generated with") in commits or PRs; imperative subject.

**Enforcement** — `hooks.json` + `protect.py`; `security-reviewer` (read-only); `/security-audit`, `/sync-context`.

## Current state

**Host Agent (`host-agent/`)** — CPU/RAM via `sysinfo` 0.39; NVIDIA GPUs via `nvml-wrapper` (Windows/Linux), graceful empty list otherwise. UI: sliders + GPU picker with exclusivity copy. Config v2 (`sharedGpuId`) in the OS app-config dir; v1 files still load. Rust Matchmaker client registers clamped caps (`POST`/`PUT /v1/hosts`); host id in `host-identity.json`. `--print-hardware` for headless dump. 46 Rust tests; clippy `-D warnings`. M1 reported 8 cores / 8 GiB / no GPU. Not built: mesh join, heartbeats, images, VMs, tray, pause, earnings.

**Matchmaker (`matchmaker/`)** — signup/login/me; host register/update (pending) + manual approve; `POST /v1/sessions` and `…/end`; camelCase JSON; `python -m matchmaker.seed` (upsert by display name, `--approve-host-id`). **37 pytest cases**. Does not call the Host Agent. `SEED_HOST_KASM_URL` is still the `example.invalid` placeholder until a real session exists. Needs `HOST_BOOTSTRAP_TOKEN` and `HOST_APPROVE_TOKEN` in `.env` to boot. How to run: `matchmaker/README.md`.

**Web (`web/`)** — Vite/React harness: signup/login, request session, open the seeded Kasm URL in a new tab, end session. Needs `VITE_MATCHMAKER_URL`. Vitest for URL/placeholder, 200/409 session mapping, and API errors. Not FR2 intent capture, not billing, not an iframe (Kasm is opened, not embedded).

**Known issues**

- Matchmaker / `web/` / Host Agent client are on local `main` after execution phase 3; `origin/main` is still Host Agent v0 + specs until someone pushes.
- Aikido MCP still fails here; use `security-reviewer` until `/aikido:setup` works.
- This Mac cannot *host*: Firecracker is Linux/KVM; Windows Standard needs QEMU+WHPX. Hosting needs a Linux or Windows PC; follow `docs/runbooks/` — do not mark GATE-* closed without a human result.

## Recent changes

Execution phase 3 adds a Rust Host Agent client that registers clamped v2 caps
with the Matchmaker phase 2 API. Hardware gates remain open. See
`PROJECT_STATUS.md`.
