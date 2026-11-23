# Known issues

Found in a code review on 2026-11-12. Ordered by severity. Remove an entry
once it is fixed and seen working on real hardware.

## 1. Every guest receives the platform's master storage key (security, high)

`POST /v1/guest/bootstrap` (`matchmaker/src/matchmaker/routers/guest.py:55`,
`guest_bootstrap.py` `bootstrap_response`) returns the single renterd S3
access key and secret from `.env` to each guest. The guest VM runs on a
volunteer's machine, so the volunteer can read the key from guest memory or
disk. With it they can read and write **every** user's bucket, which defeats
the per-user bucket design.

Fix: create a per-user (or per-session) S3 keypair limited to that user's
bucket and prefix, and hand only that to the guest. Revoke it when the
session ends.

**Fixed 2026-11-16 and checked against the real R2 bucket** (from the Mac and from the deployed server: a guest key reads and writes its own user's prefix and gets 403 on another user's prefix, on listing the bucket, and on writing with a read-only key). Not yet seen from inside a real guest.
With `STORAGE_CREDENTIALS=r2` the Matchmaker makes temporary R2
credentials limited to one bucket and one user's prefix and the guest gets
only those plus a session token. By default it signs them itself with the
parent S3 key (`storage_credentials.py` `R2LocalCredentialIssuer`,
Cloudflare's documented local signing), which needs no Cloudflare API
token. With `R2_API_TOKEN` set it asks the Cloudflare API instead
(`R2TempCredentialIssuer`); the token created on 2026-11-15 was refused
there, which is why local signing is the default.
All users share one bucket (`S3_BUCKET`), each under
`users/<user id>/files/`. A user over the quota (`STORAGE_QUOTA_GB`,
default 20) gets a read-only credential. Covered by
`tests/test_storage_credentials.py` and section 3 of the integration test.

Still open:
- R2 cannot revoke one temporary credential early, so it stays valid until
  it expires (`STORAGE_CREDENTIAL_TTL_SECONDS`, default 12 hours), not
  until session end. A host owner who copies it can read that one renter's
  files for that long.
- Files are not yet encrypted before upload (rclone crypt), so Cloudflare
  can read them.
- `STORAGE_CREDENTIALS=static` (the default, used with renterd or MinIO)
  still hands every guest the master key. Development only.
- Close this entry after a real guest mounts its prefix.

## 2. One shared host token for registration and provisioning (security, high)

All hosts share `HOST_BOOTSTRAP_TOKEN`. It authenticates host registration
on the Matchmaker, and it is also the bearer token the Matchmaker sends to a
Host Agent's `POST /v1/provision` and `POST /v1/teardown`
(`matchmaker/src/matchmaker/provision.py:237`,
`host-agent/src-tauri/src/provision_http.rs:406`). Anyone holding a friend
kit can register hosts and can pose as the Matchmaker to any Host Agent they
can reach.

Fix: keep the bootstrap token only for first registration. Issue each host
its own credential on approval and use that for Matchmaker → agent calls.
Consider rotating the bootstrap token per invite.

**Fixed in code 2026-11-15, not yet run on a real host.** Each Host Agent
now makes its own random secret on first run (`host_secret.rs`, file
`host-secret` in the app config folder) and sends it in `X-DCL-Host-Secret`
with every call. The Matchmaker binds the secret to the host id on first
use, stores only two hashes of it (`host_credentials.py`), and answers 403
to anyone else naming that host. For `/v1/provision` and `/v1/teardown` the
Matchmaker sends a token derived from that host's secret, and the agent's
listener accepts nothing else, so the shared bootstrap token can no longer
command an agent. Covered by `tests/test_host_credentials.py`, the Rust
tests in `host_secret.rs` and `provision_http.rs`, and sections 1 and 9 of
the integration test.

Still open:
- The bootstrap token is still shared and still lets its holder register
  **new** hosts (they stay pending until approved). Rotate it per invite.
- A host registered before this change has no secret until its agent next
  syncs; until then the bootstrap token alone still works for it.
- If the `host-secret` file is lost the host must register again as a new
  host.

## 3. Guest bootstrap via fw_cfg never reaches Ubuntu guests (correctness, high)

The Host Agent passes the Matchmaker origin and bootstrap token with QEMU
`-fw_cfg name=opt/org.dcl/{matchmaker,bootstrap}` (`guest_bootstrap.rs`
`qemu_bootstrap_argv`, used by `qemu_whpx.rs`, `qemu_kvm.rs`, `qemu_vfio.rs`), and
`guest-images/scripts/dcl-guest-join.py` reads them from
`/sys/firmware/qemu_fw_cfg`. The Ubuntu 24.04 cloud kernel the guest image is
built from has **no `qemu_fw_cfg` module** (it is only in
`linux-modules-extra`). Verified 2026-11-12 by booting
`release-20261017` (kernel 6.8.0-147-generic): `modprobe qemu_fw_cfg` →
"Module not found". So on QEMU hosts the guest never redeems its bootstrap:
no Headscale join, no rclone mount, no saved-as-of.

**Fixed in code 2026-11-15, not yet run in a real guest.** The Host Agent
now also passes both values as SMBIOS OEM strings
(`-smbios type=11,value=dcl.matchmaker=…,value=dcl.bootstrap=…`,
`guest_bootstrap.rs` `qemu_bootstrap_argv`; fw_cfg is still passed as a fallback),
and `dcl-guest-join.py` reads `/sys/firmware/dmi/entries/11-*/raw`, then
`dmidecode -t 11`, before fw_cfg. Covered by
`GuestJoinBootstrapSourceTests` and
`qemu_bootstrap_requires_origin_and_token_without_commas`. Close this entry
after a guest built from the Packer image joins Headscale on a real host.
The signed image has to be rebuilt to pick up the new join script.

## 3b. LAN test quick mode: desktop may not return on the second run (correctness, medium)

Under emulation (Docker, TCG) the first quick-mode run passed every check
(install, 401/401/200, KasmVNC page, clean stop, PASS report). On the second
run the VM booted (login prompt after ~85 s guest time, cloud-init finished)
but `dcl-kasmvnc.service` never answered within 40 minutes, and the console
showed no start line for it. Cause not found yet (the Mac lost internet
while reading the guest journal). Not seen on real hardware because it has
not run there. Mitigation in both apps: a timed-out later run drops the
`setup-done` marker so the next run reinstalls; the seed prints the
service status to the serial console 90 s after every boot, and the app
copies 80 console lines into the report, so the friend's report should
show the reason.

## 4. Session booking waits up to 330 s inside one HTTP request (scalability, medium)

`POST /v1/sessions` (`matchmaker/src/matchmaker/routers/sessions.py`) calls
the Host Agent synchronously and waits for the VM to boot
(`_REQUEST_TIMEOUT_SECONDS = 330.0` in `provision.py`). The request holds a
database connection and a worker thread the whole time. A few concurrent cold
boots will block the API; proxies may also time out first.

Fix: return `202 Accepted` with a `provisioning` session state, provision in
a background task, and have `web/` poll the session status.

**Fixed in code 2026-11-15.** `POST /v1/sessions` now books the host with a
`provisioning` session and returns `202`; the agent call runs on a thread
pool (`booking.py`). The browser polls `GET /v1/sessions/{id}`. A host that
fails gives a `failed` session with a reason and no charge; the credit
clock starts when the desktop is ready. Sessions stuck in `provisioning`
(for example after a restart) are failed after `PROVISION_TIMEOUT_SECONDS`
(default 420). Covered by `tests/test_async_booking.py` and section 2 of
the integration test. Still open: the pool is in-process, so bookings in
flight are lost on a Matchmaker restart (they time out and the renter must
request again). Close after the first real booking through a cold boot.

## 5. Repeated session requests create new Headscale keys (correctness, low)

When the user already has an active session, every `POST /v1/sessions` call
mints fresh Headscale preauth keys (`sessions.py:202`) instead of reusing the
existing grant. Unused keys pile up.

Fix: store the session's mesh grant and return it on repeat calls.

**Fixed in code 2026-11-15.** One guest key is minted when the session is
created and stored with the guest bootstrap row. Repeat requests return the
session without minting (`test_repeat_requests_do_not_mint_new_mesh_keys`).

## 6. Repo housekeeping (low)

- The repo is public: keep real server addresses out of it. Docs and
  examples use `lab.example.com` and `203.0.113.10`; the deploy script reads
  `DCL_DOMAIN`.
- `cargo build` prints dead-code warnings for helpers that only tests use
  (`gpu_bind.rs` live-sysfs guards, `qemu_vfio::boot_from_launch`). Mark
  them `#[cfg(test)]` or delete them.

## 7. Windows setup wrongly reports virtualization off (correctness, medium)

`Assert-FirmwareVirtualization` in `host-agent/scripts/windows-host-setup.ps1`
failed when `Win32_Processor.VirtualizationFirmwareEnabled` was `False`, which
Windows also reports whenever a hypervisor is already running (Hyper-V, WSL2,
Memory Integrity/VBS).

**Fixed 2026-11-15** (not yet re-run on Windows): the check returns early
when `Win32_ComputerSystem.HypervisorPresent` is true, as the LAN test kit
does.

## 8. Most hardware paths not yet run on real hardware (risk, high)

2026-11-14: the Windows LAN test kit (quick mode) showed a real XFCE
desktop on a second device over Wi-Fi: QEMU + WHPX on a physical Windows PC
(see `PROJECT_STATUS.md`). Same day, the Linux kit on a physical Arch PC
(QEMU + KVM) worked too, after a manual ufw rule (see 11). Still only
mocks/emulation: the signed Packer image, the Host Agent provision path end
to end, VFIO, Hyper-V DDA/GPU-P. Next: `docs/runbooks/demo.md` and
`docs/runbooks/gate-*.md`. Test logs: `docs/test-logs/`.

## 9. Windows guest disk is emulated IDE: small random reads ~62x slower (performance, high)

Windows hosts attach the guest disk with `-hda` (IDE, `/dev/sda` in the
guest), both in the LAN test (`lan-test.ps1`) and in the Host Agent
(`host-agent/src-tauri/src/qemu_whpx.rs`). Linux hosts use virtio (`/dev/vda`).
Measured 2026-11-14 (`docs/test-logs/`): random 4 KiB reads **1,426 IOPS**
on Windows vs **88,700 IOPS** on Arch; sequential **127 vs 1,206 MiB/s**.
The PCs differ, but not by that much. Users will feel it when starting
apps, opening projects, running `git`.

Fix: attach the disk as virtio on Windows too (`-drive file=…,if=virtio`;
the Ubuntu guest has the driver built in), then re-run the benchmark.

- **LAN test: fixed 2026-11-15** (`lan-test.ps1` uses virtio); not yet
  re-run on Windows hardware.
- **Host Agent: fixed 2026-11-15** (`qemu_whpx.rs` uses
  `-drive file=…,if=virtio,format=raw`); not yet run on Windows hardware.

## 10. Internet inside the guest is slow; network, not QEMU, on Arch (performance, low)

Guest downloads: ~3.5 Mbit/s on the Windows host (university Wi-Fi),
~8.2 Mbit/s on the Arch host (phone hotspot).

- **Arch: not SLIRP.** The host itself got ~8.5 Mbit/s on the same
  hotspot, so the network was the limit (2026-11-14,
  `docs/test-logs/2026-11-14-arch-lan-quick.md`). A faster network is
  needed to find SLIRP's own ceiling.
- **Windows: still unknown.** Next: run the same download on the host
  (`curl.exe -L -o NUL -w "%{speed_download} B/s" https://proof.ovh.net/files/100Mb.dat`)
  on the same Wi-Fi.

## 11. LAN test: VPN as default route breaks the link and firewall rule (correctness, medium)

On the Arch PC, WireGuard (`wg0`) carried the default route. The Linux app
took the LAN address from that route, so it opened ufw for the VPN network
and the other PC on the hotspot was blocked (fixed by hand with
`sudo ufw allow 8443/tcp`).

- **Linux: fixed** (not yet re-run on real hardware): only Wi-Fi/Ethernet
  interfaces are used, every LAN address is printed, ufw/firewalld are
  opened for each LAN network.
- **Windows: fixed 2026-11-15** (not yet re-run): only physical Wi-Fi/
  Ethernet adapters and the Mobile hotspot adapter are listed, default
  route first. The firewall rule was already fine
  (`-RemoteAddress LocalSubnet`).
- **Friend's Arch PC:** the manual rule stays until removed with
  `sudo ufw delete allow 8443/tcp`.

## 12. Copy/paste into the guest terminal is unreliable (usability, low)

Through KasmVNC into `xfce4-terminal`:
- Multi-line pastes arrive with bracketed-paste markers (`^[[200~` … `~`)
  that break the first and last command. Workaround: type
  `bind 'set enable-bracketed-paste off'` first.
- Lines pasted after an `apt-get install` are swallowed by the installer.
  Workaround: paste the install line alone.

**Fixed 2026-11-15 in quick mode** (not yet re-run): the seed turns
bracketed paste off in `/etc/inputrc` and adds a `dcl-bench` command, so
benchmarks need one typed word. **Signed image: fixed 2026-11-16** (not yet built):
`provision-base.sh` turns bracketed paste off in `/etc/inputrc`.

## 13. LAN test has no way to move files in or out of the guest (usability, low)

Only port 8443 (KasmVNC) is forwarded. Text works through the KasmVNC
clipboard panel; files do not. In quick mode files persist in the overlay
disk on the host (`C:\dcl\lan-test\quick\overlay.qcow2`,
`~/.cache/dcl-lan-test/quick/overlay.qcow2`) until the setup is redone;
signed mode (`-snapshot`) discards everything on stop. The product path
(renterd bucket mounted at `~/Persistent` by rclone) is blocked by 1 and 3.
Fix idea: forward SSH on a second LAN-only port with the one-time password
so `scp` works.

## 14. Linux LAN test: firewall rules can survive the test (correctness, medium)

After testing on the Arch PC (2026-11-14), two app rules were still in ufw:
`8443/tcp ALLOW IN 192.168.120.0/24` and `… 10.49.110.0/24`. The cleanup
on exit runs `sudo ufw delete …` with output hidden; if sudo's cached
password has expired by then (the test runs longer than sudo's timeout) or
the window is closed, the delete fails silently.

**Fixed 2026-11-15** (not yet re-run on hardware): sudo is refreshed in
the background during the test; each rule is recorded before it is added
and removed on exit; anything left is removed by
`bash lan-test.sh --cleanup`, which also lists older leftover 8443 rules.
`RUN_DIR` is deleted on exit. Covered by
`test_linux_cleanup_undoes_recorded_changes`.

## 15. Any renter on the mesh can reach any guest (security, high)

Headscale access rules are by role (`headscale/acl.hujson`): every
`tag:user` may reach every `tag:guest`, and every `tag:gateway` too.
Headscale 0.26.1 cannot add per-session rules from its policy file. So a
renter who joins the mesh with the Tailscale app (the key the Matchmaker
mints for each booking) could try to open another renter's desktop on its
mesh name. The public gateway path is scoped by its session token, but the
guest itself accepts anyone on the mesh.

Fix (both, guest firewall first):
- **Firewall inside each guest:** at boot the guest receives, with its
  bootstrap, the `100.x` address of its assigned renter (and the gateway),
  and drops every other mesh connection (nftables). Works with today's
  Headscale and protects both the mesh and the gateway paths.
- **Per-session rules in Headscale:** newer Headscale versions can keep the
  policy in their database and update it through the API; the Matchmaker
  would add and remove a rule per session. Needs checking against the
  current Headscale docs and an upgrade from 0.26.1.

**Fixed in code 2026-11-15 by taking renters off the mesh, not yet run on
real hardware.** The Matchmaker no longer mints a `tag:user` key, session
JSON carries no mesh key at all, and `headscale/acl.hujson` has no
`tag:user` rule. The gateway and its per-session token are the only way to
a guest. Covered by `test_session_json_names_the_guest_but_carries_no_mesh_key`,
the ACL test in `test_headscale.py`, and section 4 of the integration test.

Still open:
- The gateway node can still reach every guest; its scope is the
  per-session token. The guest firewall above is still worth adding.
- **Fixed 2026-11-16:** a Host Agent (or anyone who joined the mesh with a
  host kit) could open every renter's desktop, because KasmVNC ran with
  no password on the mesh. Each session now has a desktop password that
  only the guest and the gateway know (`guest_bootstrap.py`,
  `gateway/proxy.py`, `provision-kasm.sh`, `dcl-guest-join.py`); a session
  VM that never received its password stays locked. Checked against the
  real KasmVNC 1.3.4 in a container (`lan-test/e2e/kasm-auth.sh`, cases 3
  and 4) and in the integration test. Not yet seen in a real guest; the
  signed image must be rebuilt. Sunshine (Pro) is not covered.
- Hosts can still mint a mesh key before an operator approves them.
- This must be revisited when direct peer-to-peer streaming (FR17) puts
  renter devices back on a network path to guests.
- The running Headscale has to reload the new policy file on deploy.

## 16. Direct mesh connections reveal both public IPs; mesh needs an app (privacy, medium)

`100.x` mesh addresses are private, but a direct WireGuard connection is
made between the two machines' real public IPs, which the Tailscale client
knows (`tailscale status`). A renter connected over the mesh therefore
learns the host's public IP, which conflicts with the "Hide my IP" choice
(cahier FR18). Tailscale has no simple per-session "relay only" switch.
The mesh path also needs the Tailscale app on the renter's computer, so it
cannot be the Standard tier's browser-only path.

Plan: use the mesh for the Pro tier and advanced users (Moonlight is a
native app anyway), with the FR18 notice; for "Hide my IP" sessions and
browser users, use the WebRTC path with the platform relay (cahier FR17)
or the gateway.

## 17. A failed boot was reported as ready when the host had an older link (correctness, high)

Found 2026-11-15 by the integration test. The host row keeps the desktop
URL of its last session. When the agent was asked to boot a guest and
failed (HTTP error, timeout), the Matchmaker fell back to that old URL and
marked the new session ready, pointing at a desktop that no longer exists.
After a host's first successful session, every later failed boot would
have looked like a success.

**Fixed in code 2026-11-15.** A provisioner result now says whether the
agent was asked and failed (`ProvisionResult.failed`); that is always a
failed booking. Only a host with no agent at all keeps its fixed URL.
Covered by `test_a_failed_boot_is_not_hidden_by_the_previous_sessions_link`
and section 7 of the integration test.

## 18. Storage requests were signed with a date real S3 stores reject (correctness, high)

Found 2026-11-15 on the first call to the real R2 bucket. The S3 client
(`matchmaker/src/matchmaker/renterd.py`) wrote the request date as
`20261115T23:36:17Z`; the standard form has no colons. The unit tests
only checked the code against itself, so every storage call would have
failed in production.

**Fixed 2026-11-15** and confirmed against R2. Covered by
`test_sigv4_date_header_is_basic_iso8601_without_colons`.

## 19. The server could only reach a host PC through a shared SSH tunnel (design, high)

Found 2026-11-16 while deploying. To command a Host Agent behind a home
router the Matchmaker dialled a reverse SSH tunnel: every kit carried the
same private key (`tunnel.pem`) for a `dcl-tunnel` account on the server,
and each kit used one fixed port, so two Windows hosts could not be
online together. The deploy script never created that account, so on a
fresh server no host could be commanded at all.

**Fixed in code 2026-11-16, not yet run on a real host.** A Host Agent
with no `HOST_AGENT_LISTEN_ADDR` now fetches its commands itself
(`command_channel.rs`, `host_commands.py`): it polls
`POST /v1/hosts/{id}/commands/poll` every 2 s with its own secret, runs
the command through the same handler and token check as before, and
posts the result. Nothing has to reach the PC from the internet. The kits
no longer set up a tunnel unless `HOST_AGENT_URL` is given. Covered by
`tests/test_host_commands.py`, the Rust tests in `command_channel.rs`, and
section 9 of the integration test. Still open: a booking waits up to 2 s
for the next poll; the old tunnel code is still in the kits as an option.

## 20. Files in ~/Persistent would have belonged to root (correctness, high)

Found 2026-11-16 by reading the guest join script. It runs as root and
mounted the renter's storage without naming an owner, so the desktop user
(`ubuntu`) could not have opened or saved files there. The join also made
a single attempt to reach the Matchmaker, so one slow network start would
have failed the whole session, and Ubuntu 24.04 ships `fusermount3` while
rclone calls `fusermount`.

**Fixed in code 2026-11-16, not yet run in a real guest** (rebuild the
image): the mount is given to the desktop user (`--allow-other --uid
--gid --umask 077`), the join retries for about a minute, and the image
gets a `fusermount` link. Covered by `GuestDesktopPasswordTests`.

## 21. Files saved in the last seconds of a session can be lost (correctness, medium)

Open. rclone uploads a file a few seconds after it is closed. At session
end the Host Agent kills the VM at once, so a file saved just before
"End session" may never reach storage, and "saved as of" only records
the time the guest booted. Fix idea: on teardown ask the guest to shut
down cleanly (QEMU `system_powerdown`), wait for rclone to finish, then
kill; have the guest report "saved as of" when its upload queue is empty.

## 22. Deploy script problems found on the first real deploy (correctness, medium)

Found and fixed 2026-11-16 while deploying to the Oracle server:
- A fresh Ubuntu server has no Docker; the script now installs it.
- An apostrophe inside a `${VAR:?message}` swallowed the `fill` helper on
  bash 5 (macOS bash 3 did not show it).
- A redeploy did not restart the Matchmaker, so new code was not loaded.
- The operator's e-mail for certificates was written in the script; it is
  now `DCL_ACME_EMAIL` in the local `.env.vps`.
- Ubuntu 22.04 cannot run the Matchmaker (Python 3.10); use 24.04.

## 23. Every session wrote into the shared guest disk (security, high)

Found 2026-11-17 before the first Windows host install. QEMU booted the
cached `rootfs.img` read-write, and the agent checks that disk only when it
starts. The second renter on a host would have booted the first renter's
disk: browser history, cached files, the guest's mesh identity.

Fixed in code: every boot adds `snapshot=on` (KVM, WHPX, VFIO). Writes go to
a temporary file QEMU deletes on exit; the signed disk never changes. Test
`every_boot_leaves_the_signed_disk_untouched`. Not yet seen on real hardware.

## 24. Host setup failures on the first real Windows install (correctness, high)

Found and fixed 2026-11-17 on a friend's Windows 11 laptop:
- `setup.cmd` restarts itself as Administrator in a new window, which closed
  the moment setup failed, so the person saw "nothing happened". It now
  stays open, prints `SETUP FAILED: …` and logs to `C:\dcl\setup\setup.log`.
- The Administrator copy runs from `C:\dcl\setup` and tried to copy the kit
  onto itself ("cannot replace the item with itself"). It also overwrote the
  record of where `rootfs.qcow2` was.
- Setup looked for Host Agent in the wrong folders and waited 3 s. It is in
  `%LOCALAPPDATA%\Host Agent`; setup now also reads the uninstall key and
  waits up to 2 min.

Found by review, same fix round, seen working on a real Ubuntu host on
2026-11-18 (the host joined the mesh on its own): on Linux the agent
runs `tailscale up` as the desktop user, which only root may do unless that
user is the Tailscale operator. `linux-host-setup.sh` now runs
`tailscale set --operator=$USER` (Ubuntu and Arch).

## 25. Headscale refused every mesh key (correctness, high)

Found 2026-11-18: "Save and register" failed with `Headscale preauth mint
failed` (HTTP 502). The Matchmaker sent the tags as both `aclTags` and
`acl_tags`; Headscale 0.26.1 rejects that as `duplicate field "acl_tags"`.
No host and no session could have joined the mesh. The mocked test checked
that both names were sent, so it passed. Fixed: send `aclTags` only; the test
now checks that `acl_tags` is absent.

## 26. The desktop stream was refused for a missing Origin header (correctness, high)

Found 2026-11-18: the KasmVNC page loaded through the gateway, then showed
"Failed to connect to server". KasmVNC 4.0 answers 404 to a WebSocket
upgrade without an `Origin` header, and the gateway sent none. Fixed: the
gateway sends the guest its own origin (not the browser's). The test guest
now refuses a WebSocket without `Origin`, like KasmVNC.

## 27. Every desktop disconnected after about 41 seconds (correctness, high)

Found 2026-11-18. The gateway's WebSocket library pings the guest every
20 s and closes the stream when no reply comes within 20 s. KasmVNC does not
reply, so each desktop dropped with code 1006. Shown on the live guest: with
pings closed at 41 s, without pings still open at 70 s. Fixed: no keepalive
on the gateway-to-guest leg; the browser leg keeps its own. The gateway now
logs which side ended a stream, with codes only.

## 28. The agent refused the 16 GiB guest disk (correctness, high)

Found 2026-11-18: every booking failed with "verified Standard guest image is
not present". On start the agent copies the disk into its own cache, and the
copy stops at a fixed 8 GiB limit (`DEFAULT_MAX_IMAGE_BYTES`, set before any
real image existed). The Phase 4 disk is 16 GiB raw. Worse, pointing the
cache at the disk's own folder would have made the failed copy delete the
verified disk.

Worked around in the host setup scripts (Windows and Linux): set
`HOST_AGENT_IMAGE_CACHE` to the disk's folder, write the cached-image
manifest next to the disk so the agent uses it in place (it still re-hashes
it and checks the signature), and set `HOST_AGENT_IMAGE_MAX_BYTES` to
20 GiB. Proper fix, open: put the exact image size in the signed manifest and
stop the download at that size, and never delete a file the agent did not
write.

## 29. The desktop link works for anyone who has it (security, medium)

Open. Found 2026-11-18: the desktop link opens in several tabs and on
another device. It is a bearer token: anyone holding it gets the desktop
until the session ends or the token expires (12 h). Fine for testing; for the
beta, tie the link to the renter's login (a cookie bound to the account) or
make it single-use. Row 9 of the Phase 5 matrix has not been run.

## 30. A booking right after the host came online was never picked up (correctness, low)

Open. Found 2026-11-18: the first booking after the host PC came back
online expired after 45 s without being taken; the next one, 5 minutes
later, worked. The host was heartbeating, so it was offered work it did not
fetch. Not reproduced yet. Check the agent's command loop right after
start-up.

## 31. The agent boots with the sharing settings it read at start-up (correctness, high)

Fixed in code 2026-11-18, not yet on the hosts (needs a new agent build and
a reinstall): the provision gate keeps the config directory and re-reads
the saved sliders for every command. Tests
`sliders_saved_after_start_up_apply_to_the_next_boot` and
`lowering_the_sliders_after_start_up_is_obeyed` fail without the fix.

Found 2026-11-18 on the Ubuntu host: the server matched the host
with the 4 GB it had registered, and the agent refused with "this PC would
lend 3 GiB of RAM, but a session needs at least 4 GiB". The agent builds its
provision gate once when it starts (`build_provision_gate` in
`host-agent/src-tauri/src/lib.rs`) and keeps that copy of the sliders. A
change saved later reaches the Matchmaker but not the gate, so the server
and the agent disagree until Host Agent is restarted (the workaround used).
Fix: load the sharing config for each provision command (or refresh the gate
on save), and test that a saved change applies to the next boot.

## 32. The Windows agent went silent mid-session; its VM may have kept running (correctness, high)

Open. Found 2026-11-18. During a 43-minute session the Windows host's agent
sent its last poll and heartbeat at 20:46:35 UTC; the server marked the
session interrupted at 20:47:28. Tailscale on the same PC stayed connected
until 21:03:29, so the PC was awake and online: the agent itself stopped
(closed, crashed or hung). Cause unknown (see 33). Two follow-ups whatever the
cause: when the agent starts it should stop any guest VM it left behind,
and the teardown it never fetched should still be applied.

## 33. The Windows agent writes no log (operability, medium)

Open. On Linux the setup starts the agent with its output in
`host-agent.log`; on Windows it is started without one, so issue 32 could
not be explained after the fact. Fix: the agent writes a rotating log file
in its config directory on every platform.

## Accepted for now

- The gateway skips TLS certificate checks to guests (`verify=False` in
  `gateway/src/dcl_gateway/main.py`). Acceptable because that hop travels
  inside the encrypted Headscale mesh.

## 34. The Linux host's desktop kept dropping after 11 minutes (correctness, medium)

Open. Found 2026-11-18 on the Ubuntu host (2 cores, 4 GB shared). The
desktop stream held from 21:18:56 to 21:30:20 UTC, then the guest side cut
it again and again (code 1006) after 4–60 s. The guest still answered HTTP
at once, the mesh path was direct, the host kept its heartbeat, and the
Windows guest held a stream for 60 s in the same minute. Likely cause, not
confirmed: memory pressure in the 4 GB guest or on the host (no `free` or
OOM output was collected before the PC was cleaned). Next time: collect
`free -h` and `dmesg | grep -i oom` in the guest, and the host's free RAM,
and test with 6–8 GB shared.

## 35. The agent only sees NVIDIA GPUs, so a normal PC is refused Pro (correctness, high)

Fixed in code 2026-11-23, not yet on the hosts (needs a new agent build).
`gpu.rs` now lists every PCI display device (Linux sysfs, Windows
`Win32_VideoController`) and adds NVML's UUID/VRAM by PCI address. iGPUs are
flagged `integrated`: they count as the second display path and are never
offered or defaulted for dedication. IDs: Linux NVIDIA keeps its NVML UUID,
other cards use the PCI address; Windows uses the PnP instance ID for every
card, which also fixes Hyper-V never finding an NVML UUID in
`probe_powershell` (`Get-PnpDevice -InstanceId`). A saved Windows
`sharedGpuId` (old UUID) falls back to the first dedicatable card. Tests:
`linux_igpu_plus_nvidia_lists_both_with_the_card_first`,
`windows_ids_are_pnp_instance_ids_with_nvml_details`,
`an_igpu_is_never_picked_for_dedication` and others in `gpu.rs`/`models.rs`.
The Linux/Windows probe code was not compiled on the Mac (CI builds Linux).

Found 2026-11-23 reading the code before the first Pro test. GPU
detection (`host-agent/src-tauri/src/gpu.rs`) only asks NVIDIA's driver
(NVML). An Intel or AMD iGPU and any AMD card are invisible. The Pro check
(`has_second_display_path` in `host_capabilities.rs`) wants two detected
GPUs or one whose name looks integrated, so the most common Pro host, an
iGPU plus one NVIDIA card, is refused with "needs a second display path",
and an AMD-card host has no GPU at all. Fix: list PCI display devices
(Linux sysfs class `0x03`, Windows `Win32_VideoController`) for the
display-path check and for AMD cards, and keep NVML only for VRAM/UUID.
`host-agent/scripts/pro-gpu-check.{sh,ps1}` show what the agent will see.

## 36. Windows Pro tries DDA first, which client Windows does not support (correctness, medium)

Open, not yet seen on hardware. Microsoft supports Discrete Device
Assignment only on Windows Server; the friend PCs run client Windows. The
agent (`gpu_bind.rs`) picks DDA whenever the GPU has a location path, which
it always has, so GPU-P (the path client Windows supports) is never tried.
GPU-P was also designed around Windows guests that reuse the host's driver;
whether a Linux guest running Sunshine gets a usable GPU through it is not
known. Confirm on the Windows PC with `pro-gpu-check.ps1`, then try DDA
and fall back to GPU-P on failure, or pick by edition.
