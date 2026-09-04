# Phase plan — Standard Lab MVP (software)

Numbered phases below are **agent sessions**, not the cahier §11 product phases.
Cahier Phase 0 (manual Firecracker / QEMU+WHPX / Pro GPU) is a **hardware gate**,
not an agent task. Do not mark it done and do not fake a pass.

Glance file for the human: `PROJECT_STATUS.md`.
Handoffs for the next agent: `docs/phase-logs/`.

## How to run a phase

1. Read `PHASE_PLAN.md`, `PROJECT_STATUS.md`, and every file in `docs/phase-logs/`.
   Do not replay chat history.
2. Implement only the phase you are on. If it balloons, split into `N a` / `N b`
   with separate handoff files.
3. Run the relevant tests. Do not leave a broken suite.
4. Write `docs/phase-logs/phase-<N>-<slug>.md` (bullets only — see template at the
   bottom).
5. Update `PROJECT_STATUS.md` (rewrite the snapshot; do not append).
6. Commit, referencing the phase. Stop. Do not start the next phase in the same
   session.

## Constraints (every phase)

- OS-conditional Standard backend: Firecracker on Linux, QEMU+WHPX on Windows —
  a first-class branch, not a bolt-on.
- GPU passthrough exclusivity stays visible at config time wherever GPU sharing
  appears. CPU/RAM remain a concurrent slice.
- Session occupancy stays DB-level (partial unique indexes
  `sessions_one_active_per_host` / `sessions_one_active_per_user`). No app-level
  locks.
- Callers use `HostSelector.select(db, *, lab_tier) -> Host | None` via
  `get_host_selector()`. Do not import `SingleApprovedHostSelector` outside
  `matching.py` / `deps.py`.
- Do not regress Host Agent CSP, `freezePrototype`, narrowed Tauri capabilities,
  Argon2id, or JWT-from-env.
- Keep the existing 23 pytest tests passing unless you are deliberately changing
  behavior they cover — and if so, say so in the commit, the handoff, and
  `PROJECT_STATUS.md`.

## Contracts to keep stable

Matchmaker JSON is camelCase.

| Method | Path | Notes |
| --- | --- | --- |
| POST | `/v1/auth/signup` | 201 `{ accessToken, tokenType, user }` |
| POST | `/v1/auth/login` | 200, same shape |
| GET | `/v1/auth/me` | `{ id, email }` |
| POST | `/v1/hosts` | Host bootstrap Bearer; body declared caps; 201 `{ hostId, status: pending, ... }` |
| PUT | `/v1/hosts/{id}` | Host bootstrap Bearer; update caps (status unchanged); 201 if the id was new |
| POST | `/v1/hosts/{id}/heartbeat` | Host bootstrap Bearer; 200 stamps `lastSeenAt`; 404 if unknown |
| POST | `/v1/hosts/{id}/approve` | Approve Bearer (not the bootstrap token); 200 `status: approved` |
| POST | `/v1/sessions` | body `{ labTier }` (default `standard`); 201 new, 200 existing, 409 occupied/unavailable |
| POST | `/v1/sessions/{id}/end` | owner only |

Session response: `{ sessionId, hostId, labTier, startedAt, connection: { kasmUrl, kasmToken } }`.

Host register/update body: `{ displayName, labTier?, sharedLogicalCores, sharedMemoryBytes,
shareGpu, sharedGpuId, kasmUrl? }`. Host response: `{ hostId, displayName, labTier,
status, sharedLogicalCores, sharedMemoryBytes, shareGpu, sharedGpuId, kasmUrl,
lastSeenAt }`. New hosts are `pending`. `lastSeenAt` is null until a heartbeat.
`kasmToken` is not returned on host inventory.

Host Agent on-disk config is **v2**: `version`, `sharingEnabled`,
`sharedLogicalCores`, `sharedMemoryBytes`, `shareGpu`, `sharedGpuId`. v1 files
still load. Matchmaker `hosts` already has `shared_gpu_id`.

Host Agent Matchmaker calls belong in **Rust**, not `fetch` from the webview
(CSP allows no remote origins).

Host Agent provisioner (stubs, phase 5): `host_agent_lib::plan_provision(os,
request) -> Result<ProvisionPlan, ProvisionError>` in
`host-agent/src-tauri/src/provision.rs`. Linux Standard → Firecracker, Windows
Standard → QEMU+WHPX, Linux Pro → QEMU+VFIO, Windows Pro → Hyper-V GPU; macOS
is unsupported (not a Firecracker fallback). Pro requires `sharedGpuId` and
sets `gpuExclusive` plus an exclusivity warning. Never invokes a hypervisor.
Phase 6 should call this after `HostSelector.select`; do not duplicate OS
selection in the session router.

`web/` is a harness, not the product dashboard. Kasm opens in a new tab; do not
iframe it in early phases.

## Cahier map

| Cahier | This plan |
| --- | --- |
| 0 Manual compute pipeline | Hardware gates below — open, not evidenced in-repo |
| 1 MVP Standard Lab, hosts approved manually | Execution phases 1–6, then orchestrator pass |
| 2 Matching + billing + auto onboarding | Deferred |
| 3 Pro/Gamer + latency | Deferred |
| 4 Hardening | Deferred |

## Execution phases

### Phase 1 — Lock the control-plane harness on `main`

**Why next:** `matchmaker/` and `web/` are built and tested locally but **not on
`origin/main`**. Later phases cannot treat them as the baseline until they are
committed.

**Do:** Re-run pytest (expect 23) and `web` typecheck. Add automated tests for
the harness (`isLiveKasmUrl`, occupied 409, existing-session 200, API error
mapping) at the same bar as the Python suite. Commit `matchmaker/`, `web/`, and
the Python `.gitignore` entries. No new API fields.

**Do not:** Host Agent networking, provisioning, intent capture.

**Exit:** Trees on `main` (or the working branch) include Matchmaker + web;
pytest 23 pass; new web tests pass; `npm run typecheck` passes.

### Phase 2 — Host inventory registration (Matchmaker)

**Why next:** Cahier FR12. Sessions already book the one seeded host. Hosts
cannot yet declare themselves.

**Do:** Host register/update of declared caps (`sharedLogicalCores`,
`sharedMemoryBytes`, `shareGpu`, `sharedGpuId`, optional `kasmUrl`). New hosts
start **pending**; selector keeps filtering `status == approved` (manual
approval remains the Phase-1 product rule — extend the seed command or add a
minimal approve path). Occupancy stays on session indexes. Do not change
`HostSelector`'s signature.

**Do not:** Call the Host Agent. Do not auto-approve. Do not boot VMs.

**Exit:** Pytest covers pending-vs-approved, GPU id round-trip, and the original
23 cases still pass.

### Phase 3 — Host Agent → Matchmaker client (Rust)

**Why next:** Local caps are unused by the control plane.

**Do:** From Rust, register the clamped v2 config with the Matchmaker (phase 2
API). Surface GPU exclusivity in that flow. Fail if Matchmaker URL / host
credentials env vars are missing (no hardcoded secrets). Webview still does not
`fetch` the Matchmaker.

**Do not:** Heartbeats (phase 4), provision (phase 5–6), Tailscale.

**Exit:** Rust tests for payload clamping + GPU id; Host Agent typecheck still
clean. Split to 3b if tray/UI work balloons.

### Phase 4 — Heartbeats

**Why next:** Inventory is stale without liveness. Occupancy must remain session
indexes, not a host flag.

**Do:** Host Agent periodic ping; Matchmaker records last-seen. Selector may
ignore hosts whose heartbeat is stale, still through `HostSelector` — do not
inline staleness checks in the session router.

**Exit:** Tests for stale host not selected; concurrent booking still 201+409.

### Phase 5 — OS-conditional provisioner contract (stubs)

**Why next:** The Linux/Windows split must exist in code before any real
hypervisor call.

**Do:** First-class branch: Linux → Firecracker target, Windows → QEMU+WHPX
target. Pro path is a separate target that requires a dedicated GPU id and
shows exclusivity. All backends are stubs / fakes. Unit-test OS selection.

**Do not:** Invoke Firecracker, QEMU, or Kasm for real. That is the hardware
gate.

**Exit:** Tests prove the branch is not a Windows special-case bolted onto a
Linux-only path.

### Phase 6 — Matchmaker-triggered mock provision

**Why next:** Session booking currently assumes a Kasm URL already on the host
row. Cahier FR14.

**Do:** After `HostSelector.select`, call a provision port on the Host Agent
(interface, not a concrete hypervisor). Phase 5 left
`host_agent_lib::plan_provision` (`provision::plan(os, request) -> ProvisionPlan`)
as the stub; `connection_url` is `https://provision.invalid/{backend}`. Wire
that. Do not invent a second OS selector in Python. Session response shape
unchanged unless a handoff documents a new field. Comments in `sessions.py`
already mark this plug-in point.

**Do not:** Real Firecracker/QEMU. Do not put hypervisor details in the session
router.

**Exit:** Integration test: book → mock agent “ready” → `kasmUrl` from provision
result; double-book still 409.

### Phase 7 — Orchestrator MVP pass

Only after phases 1–6 each have a handoff. Read plan + status + all phase-logs.
Run Rust, pytest, and web tests. Smoke: signup → book → harness shows
connection → end. Fix cross-phase contract drift. Rewrite `PROJECT_STATUS.md`
from integration evidence. Record what is still manual (hardware gates).

## Hardware gates (human — this Mac cannot host)

| Id | What | Status to record |
| --- | --- | --- |
| GATE-FC | One Standard session: Firecracker microVM + Kasm **inside the guest**, Linux host | Open — runbook `docs/runbooks/gate-fc.md`. No human result file, guest image, or live URL |
| GATE-WHPX | One Standard session: QEMU+WHPX + Kasm inside the guest, Windows host | Open — runbook `docs/runbooks/gate-whpx.md` |
| GATE-PRO | One Pro session: GPU assignment (VFIO or Hyper-V) + Sunshine/Moonlight; host loses that GPU for the session | Open — runbook `docs/runbooks/gate-pro.md` |

An uncommitted `PROJECT_CONTEXT.md` draft once claimed Phase 0 was “done
off-Mac”. That claim is **not** treated as done: runbooks now exist, but there
is no captured Kasm URL (`SEED_HOST_KASM_URL` is still
`https://example.invalid/kasm`), no guest image, and no hypervisor scripts. A
later agent must not silently skip these gates or close them without a human.

When a gate is actually run, the operator fills the result template in the
runbook (`docs/phase-logs/gate-*-result.md`) and a human closes the gate in
`PROJECT_STATUS.md`. Do not invent a pass.

## Deferred (after phase 7)

Do not start these in an MVP phase session. Ask the human first if the work
needs a new account (Tailscale/Headscale, Storj, paid GPU cloud).

- FR2 intent prompt (“what do you want to do today?”) mapping to lab tier
- Real `HostSelector` ranking (latency, reliability, multi-host) — **swap the
  class**, do not rewrite the session router
- Billing / credits (FR7, FR16)
- Automated image pull, mesh join (FR9), tray status / pause / earnings
  (FR10–FR11)
- Storj + rclone inside the guest (FR4, FR6)
- Tailscale/Headscale per-session identity and **exit-node egress** (architecture
  §9 item 5 — legal exposure if skipped at public launch)
- Pro/Gamer automation (cahier 3)
- Image signing, reliability scoring, confidential-computing routing (cahier 4)

## Handoff template

`docs/phase-logs/phase-<N>-<slug>.md` — bullets only:

- Phase goal (1 line)
- Files created/changed
- Key decisions made and why
- Test results (pass/fail counts, what was added)
- Anything flagged for the orchestrator: open risks, assumptions, deviations
  from this plan, or contract changes other phases depend on
