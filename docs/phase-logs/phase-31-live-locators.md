# Handoff — phase 31 live locators (Kasm / Sunshine) (2026-09-07)

- Phase goal: After a successful Standard invoke or Pro GPU apply, poll until
  Kasm or Sunshine is reachable and return that live URL to Matchmaker. Do
  not occupy a host with `SEED_HOST_KASM_URL` / `provision.invalid`. Do not
  invent a public URL. Do not finish the web clickable loop.
- Verified git state at start: `8cdade1` on local `main` (phase 30). Did not
  rewrite prior commits. Did not push. Did not touch `pr/*`. Did not invent
  hardware logs. Hardware lines stay **Implemented in code. Not yet run on
  physical hardware.**

## How the live URL is discovered

- Standard (after Firecracker InstanceStart or QEMU `-accel whpx`): Kasm
  Workspaces default HTTPS **443**, URL `https://{guestMesh.hostname}/`
  (Kasm single-server docs: `https://<WEBAPP_SERVER>` after install; Packer
  does not pass `-L 8443`).
- Pro (after Linux VFIO apply or Windows Hyper-V DDA/GPU-P apply): Sunshine
  HTTPS web UI **47990**, URL `https://{guestMesh.hostname}:47990/`. Moonlight
  `{ host: mesh hostname, app: "Desktop" }`. GameStream base port is 47989;
  this phase locates the web UI, not a LAN IP.
- Hostname comes from the session Headscale guest mint (`guestMesh.hostname`,
  `dcl-guest-{sessionId}`), not the volunteer LAN. Tests inject a loopback
  HTTP origin and return **that** URL when it answers.
- Poll: HTTP GET, any status (including 401 login) counts as up. TLS failure
  falls back to TCP connect so self-signed Kasm/Sunshine certs on the mesh
  can still count. Timeout 60s production / fail-fast in tests. No
  `.invalid` success.

## When the streamer is unreachable

- Host Agent `POST /v1/provision` is **400** (`streamer at … was not reachable
  in time` or missing mesh hostname). Occupancy is not marked. Placeholder
  URLs are not returned.
- Matchmaker rejects `.invalid` locators and agent HTTP errors. If the host
  row is still a placeholder seed, the session row is deleted (occupancy
  released) and the book is **502** `host provision failed`. A live URL
  already on the host row is kept only when the agent did not answer.

## Files created/changed

- `host-agent/src-tauri/src/locator.rs` (new)
- `host-agent/src-tauri/src/provision_http.rs`, `lib.rs`
- `matchmaker/src/matchmaker/provision.py`, `routers/sessions.py`, `silence.py`
- `matchmaker/tests/test_provision.py`, `test_sessions.py`, `helpers.py`,
  `conftest.py`
- `PHASE_PLAN.md`, `PROJECT_STATUS.md`, `PROJECT_CONTEXT.md`
- `guest-images/README.md`, `host-agent/README.md`, `host-agent/.env.example`
- `matchmaker/README.md`, `docs/runbooks/gate-fc.md`, `gate-whpx.md`,
  `gate-pro.md`, architecture snippet
- `docs/phase-logs/phase-31-live-locators.md` (this file)

## Test results

- Host Agent: **320 passed**, 0 failed (`cargo test --lib` in `src-tauri`).
  Was 305 (+15).
- Matchmaker: **166 passed**, 0 failed. Was 162 (+4).
- Web: not run (untouched). Vitest still 31.
- Guest-images: not run (untouched). Unittest still 12.

New / renamed tests:

- `locator::tests::injected_loopback_http_is_returned_when_reachable`
- `locator::tests::timeout_when_streamer_never_answers`
- `locator::tests::mesh_kasm_locator_when_probe_succeeds`
- `locator::tests::mesh_sunshine_locator_when_probe_succeeds`
- `provision_http::tests::linux_standard_with_boot_inputs_invokes_and_returns_the_live_locator`
- `provision_http::tests::linux_standard_invoke_times_out_when_streamer_is_unreachable`
- `provision_http::tests::windows_standard_with_boot_inputs_invokes_whpx_and_returns_the_live_locator`
- `provision_http::tests::windows_standard_invoke_times_out_when_streamer_is_unreachable`
- `provision_http::tests::linux_pro_with_vfio_gate_applies_bind_and_returns_the_live_sunshine_locator`
- `provision_http::tests::linux_pro_vfio_times_out_when_sunshine_is_unreachable`
- `provision_http::tests::windows_pro_with_hyperv_gate_applies_dda_and_returns_the_live_sunshine_locator`
- `provision_http::tests::windows_pro_hyperv_times_out_when_sunshine_is_unreachable`
- `test_http_provisioner_accepts_a_live_locator_claimed_with_invoke`
- `test_http_provisioner_rejects_a_placeholder_locator_even_after_invoke`
- `test_book_stores_live_locator_from_agent_not_seed`
- `test_placeholder_provision_fails_the_book_and_does_not_occupy`
- `test_agent_timeout_with_placeholder_seed_fails_the_book`

Plan-only stubs (no invoke / no GPU apply) still return `.invalid`. Successful
invoke/apply does not.

## Flagged for the next agent

- **Next: web full loop** — clickable live desktop link, session
  end/history/credits against real sessions, auto-match (existing FR2/FR3)
  unless the product already chose otherwise. Do not silently switch to
  manual host browsing.
- Hardware lines stay **Implemented in code. Not yet run on physical hardware.**
- TAP/net, jailer, QEMU VFIO attach after bind, and Hyper-V `Start-VM` are
  still later. Pro discover still polls the existing `dcl-pro-guest` / mesh
  hostname after GPU apply — honest 400 if Sunshine is not up.
- `SEED_HOST_KASM_URL` remains `https://example.invalid/kasm` for the seed
  command; it is no longer stored on a successful book.
- Aikido: MCP `namespaceStatus: error` this session. Not a scan pass.
  Run `/aikido:setup`.
- `origin/main` still behind. Nothing pushed.
