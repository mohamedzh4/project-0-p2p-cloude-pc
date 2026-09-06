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
**not evidenced in this repo** (gates are **OPEN — pending real-hardware
verification**; 2026-09-05 operator PASS later retracted — no live session URL,
guest image, or measurement appendix). Self-hosted Headscale
mint+join is in (no Tailscale SaaS); default-deny ACL file + host `tag:host`
are in; session book mints ephemeral `tag:guest`/`tag:user`; guest join plans
include `--exit-node` (default `dcl-platform-exit`); live guest Tailscale is
still not executed in-repo. Persistent files use **renterd S3** (not Storj);
rclone argv/plan is locators-only. Matchmaker-triggered provision is a **stub** (fake/HTTP
port, no hypervisor).

Remote: [github.com/mohamedzh4/project-0-p2p-cloude-pc](https://github.com/mohamedzh4/project-0-p2p-cloude-pc)

## Where we actually are

From `docs/cahier-des-charges-distributed-compute-lab.md` §11.

| Phase | Content | Status |
| --- | --- | --- |
| 0 | Manual Firecracker+Kasm (Linux) and QEMU+WHPX+Kasm (Windows), plus one Pro GPU session | **OPEN — pending real-hardware verification** (2026-09-05 operator PASS later retracted). Runbooks in `docs/runbooks/`; no `gate-*-result.md`, guest image, or live Kasm URL. This Mac cannot host. |
| 1 | MVP Standard Lab, hosts approved manually | **Software loop done (execution phases 1–21):** Matchmaker auth + host inventory (pending → manual approve) + occupancy + heartbeats + internal credits ledger + silence teardown/`interrupted` + rematch via `HostSelector`; Host Agent Rust client; OS-conditional **provisioner stubs** + Matchmaker `HostProvisioner`; signed image digest+Ed25519 verify before stub boot **and** guest-image fetch to a user cache (Linux Firecracker; Windows would-fetch QEMU disk; macOS skip); launch-config JSON from clamped sliders + exclusive GPU id (never invoked); self-hosted Headscale preauth mint + Rust join (no Tailscale SaaS) + default-deny ACL file + host `tag:host` + session ephemeral `tag:guest`/`tag:user` + guest `--exit-node` join plan; renterd S3 per-user bucket + guest rclone argv/plan (not Storj); `web/` harness with FR2 intent → `labTier` and session history; `RankedHostSelector` on declared caps; Host Agent tray + immediate pause + idle/in-session/paused + accrued earnings; Pro session JSON is Sunshine/Moonlight locators (not Kasm renamed); Host Agent capability probe (OS/KVM/WHPX/IOMMU/GPU, no backend picker); hypervisor binary locate/fetch (Linux Firecracker + SHA-256 to user cache; Windows would-fetch QEMU stub). **Live desktop is still not in this repo** (`SEED_HOST_KASM_URL` stays `example.invalid`). Next: phase 22 (VFIO/DDA bind). |
| 2 | Full Matchmaker (matching + billing), automated host onboarding | Matching is `RankedHostSelector` (declared caps; no latency/reliability telemetry). Internal ledger meters sessions; payouts deferred. Silence teardown + rematch in. Plan phases 13–15. |
| 3 | Pro/Gamer Lab, latency-aware matching | Phase 16 software locators (Sunshine/Moonlight vs Kasm). Stubs only (Pro backends in `provision.rs`). GATE-PRO **OPEN — pending real-hardware verification**; no live stream in-repo. No measured latency. |
| 4 | Hardening: image signing, exit nodes, reliability scoring | Image signing software contract in (phase 13). Mesh mint+join in (phase 14). Default-deny ACL file + host tags in (14c). Session guest/user tags + guest `--exit-node` plan in (phase 17). Live egress still needs a guest image. |

**FR1** signup/login: done. **FR2** intent → `labTier`: harness mapping in `web/src/intent.ts`; Pro 409 if no GPU host. **FR3** most suitable host: `RankedHostSelector` (declared CPU then RAM; Pro dedicated GPU id). **FR8** CPU/RAM + optional dedicated GPU: done except schedule. **FR5** streamed desktop: harness can open a URL; seed is still `example.invalid`. **FR7** session history + internal credits. **FR10** idle / in-session / paused + accrued earnings (not paid out); Rust tray. **FR11** immediate pause stops heartbeats (after 45s Matchmaker treats that as silence). **FR12** inventory: caps + heartbeats; ranking uses declared caps; no load/reliability score. **FR13** partial (tier + caps; no latency). **FR14**: mock provision + heartbeats + occupied silence teardown; no live hypervisor. **FR16** metering done; payouts deferred. **FR15** software rematch on silence (same renterd bucket follows the user). **FR9** image verify + guest-image fetch/verify (phase 20; fixture bytes) + host mesh mint/join + default-deny ACL file + session ephemeral guest/user tags + guest `--exit-node` join plan (role ACLs, not pairwise; live guest still paper). **FR4** renterd per-user bucket + guest rclone argv/plan (not a live guest mount). **FR6** still paper until a guest image exists in-repo.

## Layout

```
project-0-p2p-cloude-pc/
├── docs/           architecture, cahier, runbooks, phase-logs
├── headscale/      self-hosted Headscale compose (no Tailscale SaaS)
├── renterd/        self-hosted renterd S3 gateway compose (not Storj)
├── host-agent/     Tauri tray: hardware probe, sharing UI, pause, local config
├── matchmaker/     FastAPI + Alembic + Postgres
└── web/            React harness against the Matchmaker API
```

## Architecture decisions still true today

| Decision | Status |
| --- | --- |
| Only the Matchmaker is custom glue; everything else is reused | Adopted |
| Standard Lab backend is OS-conditional: Firecracker on Linux, QEMU+WHPX on Windows | Adopted in **stubs**; hardware gates **OPEN — pending real-hardware verification** (2026-09-05 operator PASS later retracted) |
| Pro needs VFIO / Hyper-V GPU assignment — Firecracker has no PCI passthrough | Forced; Pro stubs exist, no live passthrough in this repo |
| Hosts keep using the PC. CPU/RAM is a concurrent slice; GPU is exclusive for the session | Adopted in Host Agent UI + config v2 |
| Kasm runs *inside* the guest; the hypervisor is the isolation boundary | Adopted on paper; not evidenced in-repo |
| Guest egress via platform exit nodes, never the host's home IP | Guest join plan includes `--exit-node` from `HEADSCALE_EXIT_NODE` (default overlay hostname). Mesh join does not advertise exit-node on volunteers. Live egress still needs a guest image |
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

**Host Agent (`host-agent/`)** — CPU/RAM via `sysinfo` 0.39; NVIDIA GPUs via `nvml-wrapper` (Windows/Linux), graceful empty list otherwise. Phase 18 capability probe (`host_capabilities.rs`, no Tauri): OS, KVM r/w, WHPX enum (stub off Windows), IOMMU sysfs (unknown off Linux), second-display heuristic; UI shows Standard/Pro offered and never asks which backend. Phase 19 hypervisor locate/fetch (`hypervisor.rs`): Linux Firecracker to a user cache with SHA-256 (never executed); Windows would-fetch QEMU (19b); macOS skip. Phase 20 guest image locate/fetch (`image.rs`): Linux Standard Firecracker image + optional Pro image, SHA-256 + Ed25519 before ready; Windows would-fetch; macOS skip; tests inject fetcher/`file://`/httptest (no multi-GB). Phase 21 launch config (`launch.rs`): clamped sliders → JSON for Firecracker / QEMU+WHPX / QEMU+VFIO / Hyper-V GPU; Standard omits GPU; Pro `{ id, exclusive: true }`; kernel/rootfs from phase-20 cache when present; below 2 vCPU / 4 GiB is `Err`; never invoked. UI: sliders + GPU picker with exclusivity copy, status board (paused / idle / in-session), immediate pause, accrued earnings from Matchmaker. Config v2 (`sharedGpuId`); v1 files still load. Rust Matchmaker client registers clamped caps, pings every 10s while sharing is on, and GETs host earnings. After register, optional Headscale join from Rust (`HEADSCALE_LOGIN_SERVER` + minted preauth tagged `tag:host`; never `--advertise-exit-node` or `--exit-node`). Guest rclone plan from locators or env (`GuestRcloneMount`; no S3 secrets, no spawn). Provisioner stubs: `plan_provision` (Linux→Firecracker, Windows→QEMU+WHPX, Pro→VFIO/Hyper-V GPU, macOS unsupported) plus opt-in `POST /v1/provision` that verifies SHA-256 + Ed25519 **before** the stub plan and returns Kasm vs Sunshine locators; complete storage locators yield `guestRclone`. Rust tray (no JS tray APIs). Image public key from env when listening. Cargo test count is in `PROJECT_STATUS.md`. Not built: pairwise session ACLs, live rclone mount, real VMs, payouts.

**Matchmaker (`matchmaker/`)** — signup/login/me; host register/update (pending) + manual approve; heartbeats stamp `lastSeenAt`; `RankedHostSelector` ranks declared CPU/RAM among approved live free hosts (Pro needs a dedicated GPU id); `POST /v1/sessions` calls a `HostProvisioner` after occupancy commit (body includes image digest+signature and optional storage locators + guest-mesh hint); Standard session `connection` is Kasm, Pro is Sunshine/Moonlight locators (`gpuExclusive`); silent occupied hosts are `interrupted` and rematched via `HostSelector.select`; session end writes `ledger_entries`; `GET /v1/images/{labTier}` advertises checksums; `POST /v1/hosts/{id}/mesh/preauth` mints a Headscale key tagged `tag:host`; session JSON `mesh` mints ephemeral `tag:guest`/`tag:user` with guest `--exit-node`; `POST`/`GET /v1/storage` and session JSON `storage` are per-user renterd S3 **locators** (no access keys) for rclone **inside the guest**; camelCase JSON; Alembic through `0006_user_storage_bucket`. Pytest count is in `PROJECT_STATUS.md`. `SEED_HOST_KASM_URL` is still the `example.invalid` placeholder. Needs `HOST_BOOTSTRAP_TOKEN`, `HOST_APPROVE_TOKEN`, `CREDITS_PER_MINUTE`, `IMAGE_*`, `HEADSCALE_*`, and `S3_*` (or `RENTERD_S3_*`) in `.env` to boot.

**Web (`web/`)** — Vite/React harness: signup/login, FR2 intent prompt mapped to `labTier`, request session, open a **live** Standard Kasm URL or Pro Sunshine locator in a new tab (not `.invalid`), end session, session history + credits. Needs `VITE_MATCHMAKER_URL`. Vitest for intent mapping, placeholders, 200/409 mapping, API errors, history GET, Pro vs Standard locators. Not a payment UI, not an iframe.

**Known issues**

- Matchmaker / `web/` / Host Agent are on local `main` after execution phase 21; `origin/main` is still behind until someone pushes.
- Aikido MCP may still fail here; use `/aikido:setup` if a scan is required.
- This Mac cannot *host*: Firecracker is Linux/KVM; Windows Standard needs QEMU+WHPX. GATE-FC / GATE-WHPX / GATE-PRO are **OPEN — pending real-hardware verification** (2026-09-05 operator PASS later retracted). Seed URL stays `example.invalid`. Do not invent lab output.

## Recent changes

Execution phase 21 added Host Agent launch-config generation from clamped
vCPU/RAM sliders and (for Pro) an exclusive GPU id. Kernel/rootfs paths come
from the phase-20 cache when present. No hypervisor is invoked. Gates stay
**OPEN — pending real-hardware verification**. See `PROJECT_STATUS.md`.
