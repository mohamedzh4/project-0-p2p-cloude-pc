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
  `get_host_selector()`. Do not import `RankedHostSelector` (or any concrete
  selector) in the session router. `SingleApprovedHostSelector` was replaced
  in phase 9.
- Do not regress Host Agent CSP, `freezePrototype`, narrowed Tauri capabilities,
  Argon2id, or JWT-from-env.
- Keep the Matchmaker pytest suite, Host Agent `cargo test` suite, and web
  vitest + typecheck green unless you are deliberately changing behavior they
  cover — and if so, say so in the commit, the handoff, and `PROJECT_STATUS.md`.
  Counts after phase 15: pytest TBD, cargo test TBD, vitest 27.

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
| POST | `/v1/sessions/{id}/end` | owner only; meters duration into ledger |
| GET | `/v1/sessions` | owner history + lifetime `creditsDebited` |
| GET | `/v1/hosts/{id}/earnings` | Host bootstrap Bearer; `{ hostId, creditsAccrued }` |
| GET | `/v1/images/{labTier}` | Public; `{ labTier, digestSha256, signature, algorithm, digestAlgorithm }`. `standard` / `pro` only. Not a blob download. |
| POST | `/v1/hosts/{id}/mesh/preauth` | Host bootstrap Bearer; `{ loginServer, preauthKey, user, reusable, ephemeral, expiresAt }`. Headscale mint tagged `tag:host`. Not a per-session identity. |
| POST | `/v1/storage` | User Bearer; issue per-user renterd prefix. 201 new, 200 existing. `{ provider, bucket, prefix, endpoint, region, objects }`. No access keys. |
| GET | `/v1/storage` | User Bearer; 200 same shape or 404 if never issued. |

Session response: `{ sessionId, hostId, labTier, startedAt, connection: { kasmUrl, kasmToken } }`.
End response: `{ sessionId, status, endedAt, creditsDebited }`.
History `status` is `active` | `ended` (user end) | `interrupted` (heartbeat
silence). End response is `ended` or already-`interrupted`. Occupied silent
hosts are reclaimed on book, history, heartbeat (after stamping last-seen),
and an optional background sweep (`STALE_SESSION_SWEEP_INTERVAL_SECONDS`,
default 15, `0` disables the loop). Silence reclaim adds no HTTP routes.
Rematch uses `HostSelector.select` only.

Host register/update body: `{ displayName, labTier?, sharedLogicalCores, sharedMemoryBytes,
shareGpu, sharedGpuId, kasmUrl?, agentUrl? }`. Host response: `{ hostId, displayName, labTier,
status, sharedLogicalCores, sharedMemoryBytes, shareGpu, sharedGpuId, kasmUrl,
agentUrl, lastSeenAt }`. New hosts are `pending`. `lastSeenAt` is null until a heartbeat.
`kasmToken` is not returned on host inventory. `agentUrl` is the origin Matchmaker uses
to POST the Host Agent provision port (`http://127.0.0.1:7842`). Omit it on PUT to leave
the stored value alone (same as `kasmUrl`).

Host Agent on-disk config is **v2**: `version`, `sharingEnabled`,
`sharedLogicalCores`, `sharedMemoryBytes`, `shareGpu`, `sharedGpuId`. v1 files
still load. Matchmaker `hosts` already has `shared_gpu_id`.

Host Agent Matchmaker calls belong in **Rust**, not `fetch` from the webview
(CSP allows no remote origins).

Host Agent provisioner (stubs, phase 5–6): `host_agent_lib::plan_provision(os,
request) -> Result<ProvisionPlan, ProvisionError>` in
`host-agent/src-tauri/src/provision.rs`. Linux Standard → Firecracker, Windows
Standard → QEMU+WHPX, Linux Pro → QEMU+VFIO, Windows Pro → Hyper-V GPU; macOS
is unsupported (not a Firecracker fallback). Pro requires `sharedGpuId` and
sets `gpuExclusive` plus an exclusivity warning. Never invokes a hypervisor.
The Host Agent HTTP port is `POST /v1/provision` (`provision_http.rs`); it
verifies Ed25519 over the SHA-256 digest (`image.rs`) **before** `plan` and
returns `{ connectionUrl, hypervisorInvoked, gpuExclusive }`. Body includes
`labTier`, optional `sharedGpuId`, `imageDigest`, `imageSignature`. Missing
or mismatched image metadata is 400 (no stub boot). Listen requires
`IMAGE_SIGNING_PUBLIC_KEY` and `IMAGE_LOCAL_DIGEST` (fail fast). Matchmaker
session booking, after `HostSelector.select` and occupancy commit,
calls a `HostProvisioner` port (`get_host_provisioner()`), not a hypervisor and
not a concrete selector. Origin: `host.agentUrl`, else optional
`HOST_AGENT_PROVISION_URL`. If that call is unavailable, the seeded/placeholder
`kasmUrl` on the host row is kept. Do not put Firecracker/QEMU/WHPX names in
`sessions.py`. Provision POSTs the registry digest+signature for the booked
`labTier`.

`web/` is a harness, not the product dashboard. Kasm opens in a new tab; do not
iframe it in early phases.

## Cahier map

| Cahier | This plan |
| --- | --- |
| 0 Manual compute pipeline | Hardware gates below — open, not evidenced in-repo |
| 1 MVP Standard Lab, hosts approved manually | Software: execution phases 1–7 (control plane + stubs). Live desktop still GATE-FC / GATE-WHPX |
| 2 Matching + billing + auto onboarding | Execution phases 8–12 (intent, ranking swap, tray, billing, re-match). Mesh/storage: ask human first |
| 3 Pro/Gamer + latency | Execution phase 16 + GATE-PRO |
| 4 Hardening | Execution phase 13 (images, done); phase 14 mesh mint+join; 14c default-deny ACLs + host tags + exit-node runbook (not auto-applied). Phase 15 renterd (not Storj) |

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
Run Rust, pytest, and web tests. Smoke: host registers (pending) → approve →
heartbeat → book → stub provision sets `kasmUrl` → end. Fix cross-phase
contract drift. Rewrite `PROJECT_STATUS.md` from integration evidence. Record
what is still manual (hardware gates). Extend this plan with numbered phases
for remaining cahier FRs; do not implement those FRs here.

**Exit:** Suites green; handoff `docs/phase-logs/phase-7-orchestrator-mvp.md`;
living status rewritten; GATE-* still open.

## Cahier FR gap (after phase 7)

Software MVP is the control-plane loop with stubs. Cahier FR1–FR16:

| FR | Status | Next |
| --- | --- | --- |
| FR1 signup/login | Implemented (Matchmaker + `web/`) | — |
| FR2 intent prompt → lab tier | Implemented (`web/` harness) | — |
| FR3 match to most suitable host | Partial: `RankedHostSelector` (declared CPU then RAM; Pro GPU id) | Phase 16 when latency can be measured |
| FR4 encrypted persistent files | Partial: renterd S3 prefix metadata + guest rclone runbook/stub (not Storj) | Live mount needs GATE-* + guest image |
| FR5 live streamed desktop | Partial: harness opens a URL; no live Kasm | GATE-FC / GATE-WHPX |
| FR6 logout final save + “saved as of” | Missing in the guest UI | Guest image / GATE-*; rclone runbook exists |
| FR7 session history + billing/credits | Internal ledger + `GET /v1/sessions` | Phase 11 done; no payouts |
| FR8 declare CPU/RAM slice + exclusive GPU | Implemented except availability schedule | Schedule can wait; do not break `sharedGpuId` |
| FR9 auto mesh join + signed image pull | Partial: signed digest+sig verify; host preauth mint + Rust join (no Tailscale SaaS). Default-deny ACL file + host `tag:host`. Not per-session identities. Exit-node runbook not auto-applied. Storage is renterd S3, not Storj | Live guest rclone needs GATE-* |
| FR10 tray status (idle / in-session / earnings) | Window + Rust tray; earnings from ledger | Accrued, not paid out |
| FR11 pause/stop sharing | Immediate local `sharingEnabled`; heartbeats skip | After 45s Matchmaker treats skip as silence (interrupts). No pause field |
| FR12 real-time inventory (resources, load, reliability) | Partial: caps + `lastSeenAt`; ranking uses declared caps; no load/score | Later telemetry |
| FR13 match on tier, latency, reliability | Partial: tier fit + declared caps; latency/reliability not measured | Phase 16 |
| FR14 provision, heartbeat, auto teardown | Partial: mock provision + 45s stale for new books and occupied teardown; no live hypervisor | GATE-* for real VMs |
| FR15 re-match on host disconnect | Implemented: `interrupted` + one `HostSelector.select`; 409 if none | Same renterd bucket follows the user; no Storj |
| FR16 meter usage / payouts | Metering + ledger; payouts deferred | Phase 11 metering done; no Stripe |

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

## Execution phases after 7

Do not start these in the phase-7 session. **Ask the human first** before any
phase that needs a new account (Tailscale/Headscale, paid GPU cloud). Storage
is self-hosted **renterd** (human decision 2026-09-05 — not Storj).

### Phase 8 — Intent capture (FR2)

**Do:** In `web/` (still a harness), ask “what do you want to do today?” and map
the answer to `labTier` already accepted by `POST /v1/sessions`. Keep camelCase.

**Do not:** Change `HostSelector`. Do not add billing. Do not iframe Kasm.

### Phase 9 — Ranking `HostSelector` swap (FR3, FR12 load/score slice, FR13)

**Do:** Replace `SingleApprovedHostSelector` with a ranking implementation
(tier fit, then latency/reliability when those fields exist). Inject via
`get_host_selector()`. Session router still calls
`select(db, *, lab_tier) -> Host | None` only.

**Do not:** Inline ranking in `sessions.py`. Do not import the new class there.

### Phase 10 — Host Agent tray, pause, status (FR10–FR11)

**Do:** Tray idle / in-session (earnings can be placeholder copy until phase 11).
Pause remains local `sharingEnabled` → stop heartbeats unless a later field is
designed. Keep CSP / `freezePrototype` / narrowed capabilities.

**Do not:** Real hypervisor. Do not `fetch` Matchmaker from the webview.

### Phase 11 — Session history + billing/credits (FR7, FR16)

**Do:** Metering and ledger behind the session lifecycle. Plug-in comments in
`sessions.py` already mark the spot.

**Do not:** Start this if the human has not chosen a payments approach.

### Phase 12 — Heartbeat-silence teardown + re-match (FR14 remainder, FR15)

**Do:** After ~45s of silence, end (or interrupt) the active session and re-match
the user within a bounded window. Occupancy stays on session indexes.

**Do not:** Fake a live desktop. Still no hypervisor names in `sessions.py`.

### Phase 13 — Signed base-image pipeline (FR9 image part, cahier 4 slice)

**Do:** Sign/verify guest images; Host Agent rejects a mismatch. Needs an image
registry the human can point at.

**Do not:** Close GATE-* from this work.

Done as a software contract (fixture bytes + Ed25519, no image CDN). See
`docs/phase-logs/phase-13-signed-images.md`.

### Phase 14 — Tailscale/Headscale mesh + exit-node egress (FR9 network)

**Ask human first — needs accounts / a Headscale host.** Human chose
**self-hosted Headscale** (no Tailscale cloud account).

**Do:** Compose for Headscale in-repo; Matchmaker mints a scoped preauth key
via the Headscale API; Host Agent joins from **Rust** (env login-server +
minted key). Fail fast if Headscale/Matchmaker mesh env is missing.

**Do not:** Tailscale SaaS. Do not silently claim per-session identities or
that guests already egress via a platform exit node.

Done as a software contract (mocked Headscale HTTP in tests). Default-deny
ACL file + host `tag:host` on mint, and a platform exit-node **runbook**
(not auto-applied to guests) are **14c**. See
`docs/phase-logs/phase-14-headscale-mesh.md`,
`docs/phase-logs/phase-14c-headscale-acls-exit-node.md`, and
`docs/runbooks/headscale.md`.

### Phase 14c — default-deny ACLs + platform exit-node runbook

Done. Checked-in `headscale/acl.hujson` (Headscale 0.26 `acls`). Mint tags
**`tag:host` only** — not per-session identities. Platform exit node is an
opt-in compose overlay + numbered runbook; `--exit-node` is not auto-applied.
See `docs/phase-logs/phase-14c-headscale-acls-exit-node.md`.

### Phase 15 — renterd + rclone in the guest (FR4, FR6)

**Human decision 2026-09-05: Sia renterd S3, not Storj.** (Chat said “phase
14 renterd”; Headscale already occupied 14 — this is execution **phase 15**.)
Self-hosted compose; Matchmaker httpx SigV4 against the S3 API; per-user
**bucket** (`dcl-<uuid>`) with prefix `files/`; rclone plan stub + numbered
runbook. Mount inside the guest; do not write session keys to the volunteer
host disk.

Done as a software contract (mocked renterd S3 HTTP in tests — no live
renterd / Sia network). See `docs/phase-logs/phase-15-renterd.md` and
`docs/runbooks/renterd.md`. Live guest mount and “saved as of HH:MM” still
need GATE-FC / GATE-WHPX.

### Phase 16 — Pro/Gamer automation (cahier 3)

Wire Pro backends beyond stubs (VFIO / Hyper-V GPU + Sunshine/Moonlight). Blocked
on **GATE-PRO** for a real session. Latency-aware ranking belongs in
`RankedHostSelector`, not a second matcher.

## Handoff template

`docs/phase-logs/phase-<N>-<slug>.md` — bullets only:

- Phase goal (1 line)
- Files created/changed
- Key decisions made and why
- Test results (pass/fail counts, what was added)
- Anything flagged for the orchestrator: open risks, assumptions, deviations
  from this plan, or contract changes other phases depend on
