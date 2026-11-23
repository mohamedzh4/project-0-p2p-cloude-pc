# Roadmap to a closed beta

Written 2026-11-15. This is the order of work from today to a closed beta.
Current state: [`../PROJECT_STATUS.md`](../PROJECT_STATUS.md). Open bugs:
[`issues.md`](issues.md). Tick a phase off here when its "done when" is met.

## What the beta contains

- **In the beta:** Standard tier only (CPU/RAM desktop), streamed through
  the HTTPS gateway with KasmVNC, files on Cloudflare R2, credits recorded
  but no money.
- **After the beta:** direct peer-to-peer streaming (FR17/18), Pro GPU, the
  "My files" page, Google Drive/Dropbox, payments, Storj, volunteer relays.
- **Why peer-to-peer streaming waits:** the full path has never worked on
  real machines. Adding a new streaming stack on top of an unproven base
  makes every failure harder to diagnose. The first end-to-end run comes
  first.

## The map

| Phase | Goal | Done when |
| --- | --- | --- |
| 0 | Unblock: accounts, clean repo, CI green | CI passes on GitHub; accounts exist |
| 1 | Finish must-have code and close security holes | All suites green; issues 1, 2, 4, 5, 15 closed in code. **Done 2026-11-15**, except rclone encryption |
| 2 | Integration test on one machine, no real hardware | A scripted booking reaches a desktop URL locally. **Done 2026-11-15** |
| 3 | Deploy the platform server | Public health checks answer over HTTPS. **Done 2026-11-17**, except the nightly backup |
| 4 | Build and sign the real guest image | Image boots on a KVM PC and shows a desktop |
| 5 | First real end-to-end test | A renter on another network gets a desktop and keeps their files |
| 6 | Fix and repeat until stable | Test matrix passes three times in a row on both host types |
| 7 | Beta readiness | Installers, docs, terms, monitoring, backups in place |
| 8 | Closed beta | Invited hosts and renters use it for 2–4 weeks |
| 9 | After beta | Peer-to-peer streaming, Pro, payments, storage choices |

## Phase 0: Unblock

Owner:

1. Create the Oracle Cloud Always Free account and a VM.
2. Create a Cloudflare account, one R2 bucket and an API token.
3. Rename or remove `docs/15:11:2026-status.md` (colons in a filename
   break on Windows).

Code:

4. Commit the files still modified locally.
5. Check the first CI run on GitHub and fix what fails. It has never been
   confirmed green.

## Phase 1: Complete the functionality and fix the issues

In this order; each one changes what the end-to-end test exercises.

1. **Storage on R2 with per-user keys (issue 1).** Replace renterd with R2
   in the Matchmaker and the deploy script. Each guest gets a credential
   limited to that user's bucket or prefix, revoked at session end. Add
   rclone encryption and the 20 GB quota. First check whether R2 can issue
   bucket-scoped tokens through its API; if not, fall back to short-lived
   presigned access through the Matchmaker.
2. **Per-host credentials (issue 2).** The shared token is used only for
   first registration. On approval each host gets its own secret, and the
   Matchmaker uses that one to command the agent.
3. **Session isolation (issue 15).** For the beta, renters do not join the
   mesh; the gateway is the only way to a guest. Remove the renter → guest
   rule from the ACL and stop minting renter mesh keys.
4. **Asynchronous booking (issue 4).** `POST /v1/sessions` returns at once
   with a `provisioning` state; the web page polls and shows progress.
5. **Reuse mesh keys on repeat requests (issue 5).**
6. **Web error states.** Clear messages for: no host free, host failed to
   boot, session interrupted and rematched, session ended.
7. **Host Agent diagnostics.** A "copy report" button with the last
   provision error and versions.

Done when: `bash scripts/check.sh` and CI are green, each fix has tests,
and `issues.md` is updated.

**Status 2026-11-15: done in code**, `bash scripts/check.sh` green. Not
done: rclone encryption in step 1 (R2 keys are per-user and expire, but
files are not encrypted before upload). R2 itself is untested until the
Cloudflare account exists. CI is not confirmed: nothing is pushed yet.

## Phase 2: Integration test without real hardware

Unit tests fake every boundary, so nothing yet proves the components agree
with each other.

1. One script that starts Postgres, Matchmaker, Headscale, gateway and web
   locally with Docker.
2. A Host Agent run against it using the emulated-guest harness in
   `host-agent/scripts/lan-test/e2e/`.
3. A scripted flow: sign up → register host → approve → book → poll until
   ready → fetch the desktop URL → end session → check the ledger.
4. Add it to CI if it runs in under about ten minutes.

Done when: the script passes twice in a row from a clean state.

**Status 2026-11-15: done.** `scripts/integration/run.py` (also
`bash scripts/check.sh integration`, and a CI job) passed twice in a row.
It differs from the plan in two ways. It does not need Docker: it starts
the Matchmaker and gateway as processes on any Postgres. And the Host
Agent, guest, Headscale and storage are stand-ins inside the test, not the
real Rust agent with the emulated-guest harness, because the agent is a
desktop app with no headless mode. A headless agent mode would let a later
version put the real agent in the loop.

## Phase 3: Deploy the server

1. Run `scripts/vps-deploy.sh` on the Oracle VM with the new `.env.vps`.
2. Point the four DuckDNS names at the new IP; update
   `host-agent/.env.friend`.
3. Open ports in both Oracle's security list and the VM firewall.
4. Verify: web page loads, `/api/docs` answers, Headscale health answers,
   TLS certificates issued.
5. Nightly Postgres backup to R2; test one restore.
6. Record the result in `test-logs/`.

To check first: an Always Free ARM VM needs the Docker images to build for
ARM. Not verified for this repo.

**Status 2026-11-16:** steps 1 and 2 done (an x86 `E5.Flex` VM on trial
credit, because the free ARM shape was out of capacity; DuckDNS updated;
`.env.friend` updated). Step 3 is half done: the server firewall is open,
Oracle's Security List is not. Steps 4 to 6 wait on that. Before the
trial credit ends, move to an Always Free shape or a paid plan.

## Phase 4: Build and sign the guest image

1. On the Arch PC (it has KVM), run the Packer build for the Standard image.
2. Sign it; put the digest and signature into the server's `.env`.
3. Host the image where agents can download it (R2) and set
   `HOST_AGENT_STANDARD_IMAGE_URL`.
4. Boot it by hand once with QEMU and confirm the desktop comes up.

Done when: boot time is measured and written down. That number sets the
booking timeout from Phase 1.

Status 2026-11-17:

- Steps 1–2 done, on a borrowed Ubuntu PC (6 GB RAM) instead of the Arch PC,
  with `guest-images/friend-kit/phase4.sh`. Packer took 6 min 33 s with a
  3 GB build VM. Compressed `rootfs.qcow2` is 1.2 GB. Digest `e72e4f2b…5756`,
  signature checked against the dev key. The server serves it from
  `GET /v1/images/standard`; host kit examples carry the new values.
- Step 3 not done: the disk travels on a USB drive for now (the host kits take
  `rootfs.qcow2` placed next to the setup script).
- Step 4 not done: the disk has not been booted by hand, and boot time is not
  measured yet.

Update 2026-11-18: the disk was not booted by hand, but it booted on a real
Windows host in Phase 5, and boot time is now measured: 28–44 s from
booking to desktop ready (see Phase 5). Step 3 (R2 hosting) is still open.

## Phase 5: First real end-to-end test

Setup: server on Oracle, host 1 = Arch PC, host 2 = Windows PC, renter = a
laptop or phone on mobile data, not the same Wi-Fi.

Run this matrix on each host and record every row in `test-logs/`.

| # | Scenario | Pass means |
| --- | --- | --- |
| 1 | Install Host Agent, register, approve | Host shows idle in the Matchmaker |
| 2 | Renter signs up and books | Desktop opens in the browser from another network |
| 3 | Guest joined the mesh | Guest node visible in Headscale (first real test of the SMBIOS fix) |
| 4 | Save a file, end session, book again | File is still there |
| 5 | End session | Credits recorded for renter and host; VM gone on the host |
| 6 | Host presses pause mid-session | Session interrupted, renter told, rematched if another host is free |
| 7 | Kill the host's network | Same as 6 within about a minute |
| 8 | Second renter while the host is busy | Gets the other host or a clear "none free" |
| 9 | Renter B tries renter A's desktop link | Refused |
| 10 | One-hour session | No disconnects; lag acceptable for office work |
| 11 | Host owner looks for secrets on their disk | No storage key that opens another user's files |

Expect the first attempt to fail somewhere around rows 2–4.

### Results, first run (2026-11-17 to 2026-11-18)

Setup actually used: server on Oracle, **one host only**, a friend's Windows
11 laptop (Intel i7, 15 GB RAM; shared 4 cores and 4 GB), installed from the
USB kit. Renter: the operator's Mac browser, then a phone, on a different
network from the host. The Arch PC (host 1) was not used.

| # | Result | What happened |
| --- | --- | --- |
| 1 | Pass, after fixes | Setup, register and approve worked once issues 24 and 25 were fixed. Host showed idle, heartbeat every 10 s, commands fetched every 2 s (pull mode, no router setup). |
| 2 | Pass, after fixes | Desktop opened in the browser through the gateway, also on a phone and in several tabs at once (see issue 29). Needed fixes 26, 27 and 28. |
| 3 | Pass | Each session's guest showed up in Headscale as `dcl-guest-<session>` and was removed at session end. |
| 4 | Not run | Persistent files not tested. |
| 5 | Pass, partly checked | 3 sessions ended cleanly: teardown confirmed by the host in about 2 s, guest node expired, host back to idle, ledger entries written. The host's credit share was not checked on its own. |
| 6–9 | Not run | |
| 10 | Not run | Longest session was 11 min. It held without disconnects once issue 27 was fixed. |
| 11 | Not run | |

Boot time, booking to desktop ready (host took each order in under 1 s):

| Session | Ready after | Length |
| --- | --- | --- |
| 1 | 44 s | 11 min |
| 2 | 40 s | 3 min |
| 3 | 28 s | 3 min |

Later boots are faster because Windows keeps the disk in its file cache. One
booking, the first after the host came back online, was never picked up and
expired after 45 s (issue 30).

Second host, 2026-11-18: a friend's Ubuntu PC ("Pedri", 2 cores and 4 GB
shared), installed with `linux-host-setup.sh`. Setup, register, mesh join
(as the desktop user, the issue 24 Linux fix) and approve worked on the
first try. The first three bookings failed on issue 31; after restarting
Host Agent the guest was ready **17.5 s** after booking (QEMU + KVM, versus
28–44 s on the Windows host with WHPX) and the desktop opened from another
account. The same evening the Windows host's agent went silent while a
session was running (issue 32).

Problems found and fixed during the run: issues 24–28. Still open: 21, 29,
30, 31, 32, 33, the proper fix for 28, and every row marked "Not run". Fixes are in
commit `214cbe1` and live on the server.

## Phase 6: Fix and repeat

1. Every failure becomes an entry in `issues.md` with the log.
2. Fix, add a test that would have caught it, rerun the matrix.
3. Re-run the LAN kit on both PCs to close issues 3b, 9, 11, 12 and 14.

Done when: the full matrix passes three times in a row on both hosts, on
at least two different days.

## Phase 7: Beta readiness

Product:

1. Installers from CI for Windows and Linux. Windows shows a SmartScreen
   warning without a code-signing certificate; for a closed beta a note in
   the instructions is enough.
2. A one-page host guide and a one-page renter guide.
3. Invite-only signup, and a cap on hosts and session length.
4. An admin view or script for approving hosts and seeing active sessions.

Operations:

5. Uptime check on the server with an alert.
6. Logs searchable by session id.
7. A kill switch: one command that stops new bookings.
8. Rate limits on signup, login and booking.

Legal (owner; short versions are fine for a closed beta):

9. Terms of use, privacy notice and a host agreement. Guest internet
   traffic leaves through the platform server, so renter abuse lands on
   that IP. The terms and a traffic policy on the exit node must cover it.

## Phase 8: Closed beta

1. 3–5 known hosts and 10–20 renters, for 2–4 weeks.
2. One feedback channel.
3. Track weekly: share of bookings that reach a desktop, time from booking
   to desktop, share of sessions interrupted, reported lag.
4. Set the targets for widening the beta before it starts.

## Phase 9: After the beta

In the likely order of demand: direct peer-to-peer streaming with the
privacy choice (FR17/18), the "My files" page (FR20), Pro GPU on real
hardware, payments, Google Drive/Dropbox, then Storj and volunteer relays.
Details for each are in `PROJECT_STATUS.md`.

## Who does what

- **Owner only:** Oracle and Cloudflare accounts, DNS, the physical PCs,
  running the real tests, inviting people, legal wording.
- **Code work:** Phases 1 and 2, deploy script changes, image build steps,
  fixes in Phase 6, installers and operations tooling in Phase 7.
