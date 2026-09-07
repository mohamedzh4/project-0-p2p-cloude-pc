# Phase plan — Standard Lab MVP (software)

Numbered phases below are **agent sessions**, not the cahier §11 product phases.
Hardware status is three honest lines in `PROJECT_STATUS.md` (never “confirmed
working” until a human reports a live session on metal). This Mac cannot host.
Do not invent lab output. Guest disks: `guest-images/`.

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
  Counts after phase 26: pytest 161, cargo test 246, vitest 31, guest-images
  unittest 12.

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
| POST | `/v1/hosts/{id}/mesh/preauth` | Host bootstrap Bearer; `{ loginServer, preauthKey, user, reusable, ephemeral, expiresAt }`. Headscale mint tagged `tag:host`. Not a guest/user identity. |
| POST | `/v1/storage` | User Bearer; issue per-user renterd bucket `dcl-<uuid>` + prefix `files/`. 201 new, 200 existing. `{ provider, bucket, prefix, endpoint, region, objects, pathStyle, mountInsideGuest, rclone, rcloneArgs, envAuth }`. No access keys. |
| GET | `/v1/storage` | User Bearer; 200 same shape or 404 if never issued. |

Session response: `{ sessionId, hostId, labTier, startedAt, connection, storage, mesh }` where `storage` is the same locator object as `/v1/storage` (no access keys). `mesh` is `{ loginServer, exitNode, exitNodeApplied, guest, user }` with ephemeral Headscale preauth keys tagged `tag:guest` / `tag:user` and guest `tailscaleUp` including `--exit-node` when `HEADSCALE_EXIT_NODE` is set (default `dcl-platform-exit`). Host Agent join stays the preauth route (`tag:host`, never `--exit-node`). Standard `connection` is `{ streamer: "kasm", kasmUrl, kasmToken, sunshineUrl: null, moonlight: null, gpuExclusive: false }`. Pro `connection` is `{ streamer: "sunshine", kasmUrl: null, kasmToken: null, sunshineUrl, moonlight: { host, app }, gpuExclusive: true }` — not Kasm renamed. Locators still live in `hosts.kasm_url`. Pro-GPU: implemented in code, not yet
run on physical hardware. Provision still returns stub locators; there is no
live Sunshine/Moonlight session in this repo.
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
returns `{ connectionUrl, hypervisorInvoked, gpuExclusive, streamer }` plus
Pro-only `sunshineUrl` and `moonlight: { host, app }`. Standard `streamer` is
`kasm`. Pro `streamer` is `sunshine` (stub origin `https://provision.invalid`).
Body includes `labTier`, optional `sharedGpuId`, `imageDigest`, `imageSignature`.
Missing or mismatched image metadata is 400 (no stub boot). Listen requires
`IMAGE_SIGNING_PUBLIC_KEY` and `IMAGE_LOCAL_DIGEST` (fail fast). Matchmaker
session booking, after `HostSelector.select` and occupancy commit,
calls a `HostProvisioner` port (`get_host_provisioner()`), not a hypervisor and
not a concrete selector. Origin: `host.agentUrl`, else optional
`HOST_AGENT_PROVISION_URL`. If that call is unavailable, the seeded/placeholder
locator on the host row is kept (`kasmUrl` column; session JSON still splits
Kasm vs Sunshine by `labTier`). Do not put Firecracker/QEMU/WHPX names in
`sessions.py`. Provision POSTs the registry digest+signature for the booked
`labTier`.

`web/` is a harness, not the product dashboard. Kasm opens in a new tab; do not
iframe it in early phases.

## Cahier map

| Cahier | This plan |
| --- | --- |
| 0 Manual compute pipeline | Hardware: three honest lines in `PROJECT_STATUS.md`. Not yet run on physical hardware. Guest disks: `guest-images/` |
| 1 MVP Standard Lab, hosts approved manually | Software: execution phases 1–7 (control plane + stubs). Live desktop is still not in this repo (`SEED_HOST_KASM_URL` stays `example.invalid`) |
| 2 Matching + billing + auto onboarding | Execution phases 8–12 (intent, ranking swap, tray, billing, re-match). Mesh/storage: ask human first |
| 3 Pro/Gamer + latency | Execution phase 16 software (Sunshine locators). Pro-GPU implemented in code, not yet run on physical hardware; no live stream in-repo |
| 4 Hardening | Execution phase 13 (images); 14 mesh mint+join; 14c ACLs + host tags; 15 renterd; 17 session guest/user tags + guest `--exit-node` plan + guest rclone argv; 26 Packer guest images. Live desktop / pairwise ACLs still paper |

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
living status rewritten; hardware still not run on physical hardware.

## Cahier FR gap (after phase 7)

Software MVP is the control-plane loop with stubs. Cahier FR1–FR16:

| FR | Status | Next |
| --- | --- | --- |
| FR1 signup/login | Implemented (Matchmaker + `web/`) | — |
| FR2 intent prompt → lab tier | Implemented (`web/` harness) | — |
| FR3 match to most suitable host | Partial: `RankedHostSelector` (declared CPU then RAM; Pro GPU id) | Phase 16 when latency can be measured |
| FR4 encrypted persistent files | Partial: renterd S3 locators + guest rclone argv/plan (env auth, not Storj). Provision hook returns `guestRclone` from locators. Phase 26 Packer image installs rclone | Live mount still needs a booted guest |
| FR5 live streamed desktop | Partial: harness opens a URL; seed is still `example.invalid`. Phase 26 builds Kasm/Sunshine disks | Standard-Linux / Standard-Windows implemented in code, not yet run on physical hardware |
| FR6 logout final save + “saved as of” | Missing in the guest UI | rclone argv/plan exists; needs a booted guest |
| FR7 session history + billing/credits | Internal ledger + `GET /v1/sessions` | Phase 11 done; no payouts |
| FR8 declare CPU/RAM slice + exclusive GPU | Implemented except availability schedule | Schedule can wait; do not break `sharedGpuId` |
| FR9 auto mesh join + signed image pull | Partial: signed digest+sig verify; host preauth mint + Rust join (`tag:host`, no Tailscale SaaS). Session book mints ephemeral `tag:guest` + `tag:user`. Guest join plan includes `--exit-node` from `HEADSCALE_EXIT_NODE`. ACL file is still role-based (not pairwise session tags). Storage is renterd S3, not Storj | Live guest Tailscale/rclone still need a guest image |
| FR10 tray status (idle / in-session / earnings) | Window + Rust tray; earnings from ledger | Accrued, not paid out |
| FR11 pause/stop sharing | Immediate local `sharingEnabled`; heartbeats skip | After 45s Matchmaker treats skip as silence (interrupts). No pause field |
| FR12 real-time inventory (resources, load, reliability) | Partial: caps + `lastSeenAt`; ranking uses declared caps; no load/score | Later telemetry |
| FR13 match on tier, latency, reliability | Partial: tier fit + declared caps; latency/reliability not measured | Do not fake `latency_ms` |
| FR14 provision, heartbeat, auto teardown | Partial: mock provision + 45s stale for new books and occupied teardown; no live hypervisor | Real VMs are still not in this repo |
| FR15 re-match on host disconnect | Implemented: `interrupted` + one `HostSelector.select`; 409 if none | Same renterd bucket follows the user; no Storj |
| FR16 meter usage / payouts | Metering + ledger; payouts deferred | Phase 11 metering done; no Stripe |

## Hardware status (human — this Mac cannot host)

Status is one honest line each in `PROJECT_STATUS.md`. Never write “confirmed
working” until a human reports a live session on metal. Do not invent lab
output. Pointers: `docs/runbooks/gate-fc.md`, `gate-whpx.md`, `gate-pro.md`.

- Standard-Linux: Implemented in code. Not yet run on physical hardware.
- Standard-Windows: Implemented in code. Not yet run on physical hardware.
- Pro-GPU: Implemented in code. Not yet run on physical hardware.

`SEED_HOST_KASM_URL` is still `https://example.invalid/kasm`. Guest-image
**build** is `guest-images/` (phase 26). There is no gate-result template.
Software phases may proceed without a metal run.

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

**Do not:** Tailscale SaaS. Do not silently claim pairwise session ACLs or
that guests already egress live via a platform exit node.

Done as a software contract (mocked Headscale HTTP in tests). Default-deny
ACL file + host `tag:host` on mint are **14c**. Session guest/user tags and
guest `--exit-node` join plans are **phase 17**. See
`docs/phase-logs/phase-14-headscale-mesh.md`,
`docs/phase-logs/phase-14c-headscale-acls-exit-node.md`,
`docs/phase-logs/phase-17-complete-software.md`, and
`docs/runbooks/headscale.md`.

### Phase 14c — default-deny ACLs + platform exit-node runbook

Done. Checked-in `headscale/acl.hujson` (Headscale 0.26 `acls`). Host mint
tags **`tag:host`**. Phase 17 mints ephemeral `tag:guest` / `tag:user` on
book. Platform exit node overlay is the guest-path compose; guest join
plans include `--exit-node` (default `dcl-platform-exit`). See
`docs/phase-logs/phase-14c-headscale-acls-exit-node.md` and
`docs/phase-logs/phase-17-complete-software.md`.

### Phase 15 — renterd + rclone in the guest (FR4, FR6)

**Human decision 2026-09-05: Sia renterd S3, not Storj.** (Chat said “phase
14 renterd”; Headscale already occupied 14 — this is execution **phase 15**.)
Self-hosted compose; Matchmaker httpx SigV4 against the S3 API; per-user
**bucket** (`dcl-<uuid>`) with prefix `files/`; rclone plan stub + numbered
runbook. Mount inside the guest; do not write session keys to the volunteer
host disk.

Done as a software contract (mocked renterd S3 HTTP in tests — no live
renterd / Sia network). Phase 17 strengthens guest rclone argv (`rcloneArgs`,
`--s3-env-auth`) and the Host Agent provision hook (`guestRclone` from
locators). Live guest mount and “saved as of HH:MM” still need a guest image
in-repo (not in this tree). See `docs/phase-logs/phase-15-renterd.md` and
`docs/runbooks/renterd.md`.

### Phase 16 — Pro/Gamer automation (cahier 3)

Done as a **software contract**. Session JSON and the Host Agent provision
port distinguish Standard Kasm from Pro Sunshine/Moonlight. GPU exclusivity
stays a Pro matching filter (`shareGpu` + non-empty `sharedGpuId`) plus
`gpuExclusive` on the connection. VFIO / Hyper-V paths remain stubs.
Pro-GPU is implemented in code, not yet run on physical hardware — this phase
still did not invoke a hypervisor or a live stream, and none is in the repo. Latency-aware
ranking still belongs in `RankedHostSelector` when a measured value exists; do
not fake `latency_ms`. See
`docs/phase-logs/phase-16-pro-gamer.md`.

### Phase 17 — remaining mesh/storage guest path (software)

Done as a **software contract** without a second host PC and without new
paid accounts. Session book mints ephemeral Headscale `tag:guest` +
`tag:user` keys (0.26 `aclTags`); guest `tailscaleUp` includes
`--exit-node=$HEADSCALE_EXIT_NODE` (default overlay hostname
`dcl-platform-exit`). Host Agent join still never advertises or uses an
exit node. Guest rclone argv/plan is locators-only (`--s3-env-auth`);
provision may return `guestRclone` from bucket/endpoint locators and still
does not mount on the volunteer OS.

**Leftover (not numbered phases):** Headscale 0.26 file policy cannot mint
pairwise `tag:session-<uuid>` ACL owners — role tags remain. No guest image
in-repo, so Tailscale/rclone plans are not executed inside a VM. FR13 still
waits on a measured latency — do not fake `latency_ms`. No Stripe/payouts,
Storj, or Tailscale SaaS.

See `docs/phase-logs/phase-17-complete-software.md`.

### Phase 18 — Host capability probing

**Why next:** Hosting eligibility must be a probe result (OS + KVM / Windows
hypervisor / IOMMU / GPU / second display path), not a question in the UI and
not a closed hardware gate. This Mac and Linux-without-KVM CI must stay
testable via injected probe results.

**Do:** In Host Agent Rust, a Tauri-free module that reports:

- OS: Linux / Windows / macOS-unsupported (`HostOs`)
- Linux: `/dev/kvm` present and readable/writable (path injectable in tests)
- Windows: WHPX/Hyper-V as a probe enum (non-Windows CI stubs
  `notApplicable` / `unknown`)
- GPU list + heuristic for a second display path (iGPU or 2+ GPUs)
- Pro eligibility: IOMMU/VFIO-ish flags optional/unknown when not Linux with
  IOMMU sysfs

Surface Standard/Pro **offered** plus a plain-language reason in the Host
Agent window. Never ask the user which backend. Keep GPU exclusivity copy.
Do not change `HostSelector.select`, occupancy indexes, CSP, Argon2id, or JWT.

**Do not:** Invoke Firecracker, QEMU, or Kasm. Do not invent live URLs.
`SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`.

**Exit:** Mac and Linux-without-KVM → Standard/Pro `offered=false` with a
reason. Two GPUs → second display path true. Existing pytest / cargo / vitest
suites stay green. Handoff `docs/phase-logs/phase-18-capability-probe.md`.

### Phase 19 — Automated hypervisor binary fetch + log

**Why next:** Phase 18 knows whether this PC can host. The next software slice
is fetching the right hypervisor **binary** for the probed OS and recording
the path/version — still without booting a guest.

**Do:** After a successful capability probe, download or locate Firecracker
(Linux) or QEMU (Windows) using the OS the probe already chose. Log version
and install path. Fail fast if the probe said Standard is not offered. Keep
the OS split first-class (not a Windows `if` on a Linux-only path).

**Do not:** Invoke the hypervisor. Do not ask the user which backend. Do not
close GATE-FC / GATE-WHPX. Do not change `HostSelector.select` or occupancy
indexes.

**Exit:** Tests cover fetch/log on injected paths; no real boot; suites green.

Done as a **software contract** (2026-09-06). Linux Standard locates or fetches
Firecracker into a user cache (`~/.cache/dcl-host-agent/hypervisors` or
`HOST_AGENT_HYPERVISOR_CACHE`) from a documented GitHub release URL, SHA-256
verified, never executed. Windows Standard fetches the pinned Weilnetz NSIS
installer, verifies the published SHA-512, and runs `/S` `/D=<cache>` (phase
24 / leftover **19b**). macOS skips with a logged reason. Tests inject a fetcher,
`file://`, or loopback httptest; non-loopback HTTP is refused under `cfg(test)`
unless `HOST_AGENT_HYPERVISOR_NETWORK=1`. GATE-* stay OPEN. See
`docs/phase-logs/phase-19-hypervisor-fetch.md` and
`docs/phase-logs/phase-24-windows-qemu-silent-install.md`.

**19b (done in phase 24):** Windows QEMU silent install with checksum. UAC
consent remains the only manual step (`qemu.nsi` `RequestExecutionLevel admin`).

### Phase 20 — Guest image fetch/verify (extend `image.rs`)

**Why next:** Phase 13 already verifies Ed25519 over a SHA-256 digest for
fixture bytes. Real hosting needs to **fetch** a guest image and reuse that
verify before any boot.

**Do:** Extend `image.rs` (and Matchmaker `GET /v1/images/{labTier}` as
needed) to fetch a guest image to a local cache and verify digest+signature
before it is eligible for launch. Keep the public-key-only contract. Fail
fast on mismatch.

**Do not:** Invent a live Kasm URL. Do not put a signing secret in the agent
or Matchmaker runtime. Do not close hardware gates. Do not boot the image.

**Exit:** Tests cover fetch + verify success/mismatch; existing image fixture
tests stay green.

Done as a **software contract** (2026-09-06). Linux Standard fetches the signed
Firecracker guest image into a user cache (`HOST_AGENT_IMAGE_CACHE` or
`~/.cache/dcl-host-agent/images`) and verifies SHA-256 + Ed25519 against
Matchmaker `IMAGE_*` metadata **before** the file is treated as ready.
Mismatch fails fast and does not leave a ready disk. Linux Pro (when offered)
fetches the Pro image the same way. Windows Standard logs **would fetch** a
QEMU disk (not Firecracker). macOS skips with a logged reason. Tests inject a
fetcher, `file://`, or loopback httptest; non-loopback HTTP is refused under
`cfg(test)` unless `HOST_AGENT_IMAGE_NETWORK=1`. No private key in the agent.
The webview does not fetch. GATE-* stay OPEN. See
`docs/phase-logs/phase-20-guest-image-fetch.md`.

### Phase 21 — Generate launch config from sliders + GPU id

**Why next:** Declared CPU/RAM sliders and `sharedGpuId` must become a
hypervisor **config**, not a live boot, so later phases have a stable input.

**Do:** Map clamped v2 sharing caps (and Pro `sharedGpuId`) to a launch
config for the backend the probe already selected. Standard stays a CPU/RAM
slice. Pro keeps GPU exclusivity (host loses that GPU for the session). Do
not invent a second OS selector in Python. Do not put Firecracker/QEMU/WHPX
names in `sessions.py`.

**Do not:** Invoke the hypervisor. Do not weaken exclusivity copy. Do not
change occupancy indexes or `HostSelector.select`.

**Exit:** Tests: slider values appear in the config; Pro requires GPU id;
macOS still unsupported.

Done as a **software contract** (2026-09-06). `launch.rs` maps clamped v2
sliders onto the stub backend already selected in `provision.rs` (Linux
Standard → Firecracker, Windows Standard → QEMU+WHPX, Linux Pro → QEMU+VFIO,
Windows Pro → Hyper-V GPU). Standard omits GPU assignment even if an id is
stored. Pro includes `{ id, exclusive: true }` when `shareGpu` is on; GPU is
never a concurrent slice. Kernel/rootfs paths come from the phase-20 cache
(`vmlinux` / `rootfs.img`) when present. Below the documented Kasm minimum
(2 vCPU / 4 GiB) returns a plain-language `Err` (phase 23 owns full UX).
macOS is still unsupported. Never invokes a hypervisor. See
`docs/phase-logs/phase-21-launch-config.md`.

### Phase 22 — VFIO/DDA bind with rollback

**Why next:** Pro on metal needs exclusive GPU assignment with a way back.
Firecracker has no PCI passthrough — this is QEMU/KVM+VFIO or Hyper-V DDA,
only when the phase-18 probe offered Pro.

**Do:** Bind the dedicated GPU for the session and roll back on failure so
the host display path is restored. Refuse if the probe said no second
display path. Keep exclusivity visible. Linux and Windows are peer paths.

**Do not:** Close GATE-PRO. Do not invent `nvidia-smi` / PCI / Moonlight
output. Do not use Firecracker for Pro. Do not change `HostSelector`.

**Exit:** Tests cover bind + rollback on mocks; no live VFIO required on this
Mac.

Done as a **software contract** (2026-09-06). Linux Pro plans vfio-pci bind
(unbind host driver → bind vfio-pci) from injectable sysfs/driver paths so
tests never touch real devices. Apply rolls back to the previous driver on
failure. Missing IOMMU or a second display path refuses before any write.
PCI addresses are resolved from sysfs (UUID / BDF), not `lspci`. Windows
Pro logs a **DDA plan** from the window (no PowerShell). Apply (leftover
**22b**, done in **phase 25**) runs documented Hyper-V/PnP cmdlets through
an injectable runner with rollback. Firecracker is never used for Pro. GPU
exclusivity copy stays at config time. GATE-PRO stays OPEN. See
`docs/phase-logs/phase-22-vfio-bind.md` and
`docs/phase-logs/phase-25-hyperv-dda-gpu-p.md`.

**22b (done in phase 25):** live Hyper-V DDA / GPU-P apply via injectable
PowerShell/CIM, hang-safe timeout, rollback on mid-step failure.

### Phase 23 — Insufficient-resource rejection UX + tests

**Why next:** If the probe says Standard/Pro is not offered, or sliders
exceed what this PC can spare, the host should see a rejection — not a stub
boot and not a backend picker.

**Do:** Host Agent UX + tests when KVM/WHPX is missing, when there is no
second display path for Pro, or when declared caps cannot be met. Reuse
phase-18 reasons. GPU exclusivity copy stays.

**Do not:** Invoke a hypervisor. Do not invent lab output. Do not change
Matchmaker occupancy or `HostSelector.select`.

**Exit:** Tests for Mac / Linux-without-KVM / no second display path; pytest
and vitest unchanged unless a documented contract changes.

Done as a **software contract** (2026-09-06). `eligibility.rs` wires the
phase-18 probe, the 2 vCPU / 4 GiB / 50 GiB floor, and a refused VFIO/DDA
plan into one Standard/Pro offer. The window shows **Available** vs **Not
available** with the reason. A blocked tier is not offered: launch config
does not emit, and `POST /v1/provision` with the live gate returns 400
(plain-language `detail`, no stub URL, occupancy unset). GPU exclusivity
copy stays when Pro *is* offered. Unknown spare disk does not block.
macOS / Linux-without-KVM / no second display path / below-minimum sliders
are covered by unit tests. Leftover **19b** (Windows QEMU silent install) was
done in **phase 24**. Leftover **22b** (live Hyper-V DDA/GPU-P PowerShell)
was done in **phase 25**. GATE-* stay OPEN. See
`docs/phase-logs/phase-23-insufficient-resource-ux.md`.

### Phase 24 — Windows QEMU silent/unattended install (leftover 19b)

**Why next:** Phase 19 located Firecracker on Linux but only logged
**would fetch QEMU** on Windows. GATE-WHPX still needs the installer to
run with the flags the official NSIS setup actually supports.

**Do:** Detect missing QEMU on Windows, fetch the documented Weilnetz
installer (SHA-512), run NSIS `/S` and `/D=<dir>` (last, unquoted). Keep
the injectable fetcher/installer pattern so CI never hits the QEMU CDN.
Document UAC if it cannot be suppressed. Do not boot a VM.

**Do not:** Close GATE-WHPX. Do not invent WHPX/QEMU lab logs. Do not
start leftover **22b** (Hyper-V DDA). Do not execute `qemu-system-*.exe`.

**Exit:** Tests prove silent argv on an injected installer; non-zero
installer is not treated as installed; suites green.

Done as a **software contract** (2026-09-06). Windows Standard fetches
`qemu-w64-setup-20260811.exe` (or the ARM sibling) from qemu.weilnetz.de,
verifies the published SHA-512, and runs the NSIS installer with `/S`
`/D=<user cache>`. Tests inject a fake installer (no real download).
`qemu.nsi` `RequestExecutionLevel admin` means Windows UAC is the only
remaining manual step — `/S` does not suppress it. The hypervisor is
never invoked. GATE-* stay OPEN. See
`docs/phase-logs/phase-24-windows-qemu-silent-install.md`.

### Phase 25 — Hyper-V DDA / GPU-P apply with rollback (leftover 22b)

**Why next:** Phase 22 bound Linux VFIO on a mock sysfs tree with rollback,
but Windows only logged **would assign**. GATE-PRO still needs the
documented Hyper-V cmdlets to run when hardware is eligible.

**Do:** When the probe offers Pro, execute exclusive DDA (preferred) or
GPU-P (full partition only) through an injectable PowerShell/CIM runner.
Roll back mid-step failures. Do not boot a guest. Do not spawn live
PowerShell from tests.

**Do not:** Close GATE-PRO. Do not invent nvidia-smi / WHPX / Firecracker
boot logs. Do not rebase existing `pr/*` branches. Do not push `main`.

**Exit:** Tests cover assign success and assign-fail → rollback on a fake
runner; `cargo test --lib` green.

Done as a **software contract** (2026-09-06). Windows Pro apply prefers
DDA (`Disable-PnpDevice`, `Dismount-VMHostAssignableDevice -Force`,
`Add-VMAssignableDevice`) and falls back to exclusive GPU-P
(`Set-VMHostPartitionableGpu -PartitionCount 1`,
`Add-VMGpuPartitionAdapter`, `Set-VMGpuPartitionAdapter` at 100000000).
Production uses `ProcessHyperVGpuOps` (`powershell.exe -NoProfile
-NonInteractive`, 30s kill). Tests inject a fake runner. The window stays
plan-only (`apply: false`). Not yet run on physical hardware. See
`docs/phase-logs/phase-25-hyperv-dda-gpu-p.md`.

Numbered leftover **22b** is closed.

### Phase 26 — Guest image pipeline (Packer)

**Why next:** Phases 13 and 20 verify and fetch a signed disk, but the repo
had no way to *produce* a bootable guest with Kasm (Standard) or Sunshine
(Pro). Nothing else has a real disk to boot.

**Do:** Packer + QEMU templates that emit a signed `rootfs.img` / `rootfs.qcow2`
the Host Agent can fetch. Standard installs Kasm Workspaces inside the guest
(official `install.sh --accept-eula --admin-password --user-password`). Pro
installs Sunshine from the official Ubuntu 24.04 `.deb` (`dpkg -i`) and enables
the documented user unit. Sign Ed25519 over the raw SHA-256 with a **dev** key
from `IMAGE_SIGNING_PRIVATE_KEY`. Checksums + signatures sit next to the image.
Do not commit ISOs, PEMs, or disks. Do not invoke a hypervisor from Host Agent.

**Do not:** Firecracker boot, QEMU `-accel whpx` boot, live VFIO sysfs from the
window, live locator return, or the web loop. Those are later phases.

**Exit:** Operator can run `guest-images/scripts/build.sh standard|pro` with
documented inputs (cloud image URL or local path, Packer/QEMU, dev PEM).
Tests cover templates + signing helper without hardware. Suites green.

Done as a **pipeline in-repo** (2026-09-07). Artifacts are gitignored. Linux
fetch/verify already existed; Windows remains would-fetch. Host Agent still
does not boot. See `guest-images/README.md` and
`docs/phase-logs/phase-26-guest-image-pipeline.md`.

**Next:** Firecracker real invoke from the phase-21 launch config, using a
disk this pipeline produced. Still not WHPX boot, live VFIO, or the web loop.

## Handoff template

`docs/phase-logs/phase-<N>-<slug>.md` — bullets only:

- Phase goal (1 line)
- Files created/changed
- Key decisions made and why
- Test results (pass/fail counts, what was added)
- Anything flagged for the orchestrator: open risks, assumptions, deviations
  from this plan, or contract changes other phases depend on
