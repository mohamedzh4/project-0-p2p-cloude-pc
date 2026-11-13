# Known issues

Found in a code review on 2026-11-12. Ordered by severity. Remove an entry
once it is fixed.

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

## 3. Guest bootstrap via fw_cfg never reaches Ubuntu guests (correctness, high)

The Host Agent passes the Matchmaker origin and bootstrap token with QEMU
`-fw_cfg name=opt/org.dcl/{matchmaker,bootstrap}` (`guest_net.rs`
`qemu_fw_cfg_argv`, used by `qemu.rs` and `qemu_kvm.rs`), and
`guest-images/scripts/dcl-guest-join.py` reads them from
`/sys/firmware/qemu_fw_cfg`. The Ubuntu 24.04 cloud kernel the guest image is
built from has **no `qemu_fw_cfg` module** (it is only in
`linux-modules-extra`). Verified 2026-11-12 by booting
`release-20261017` (kernel 6.8.0-147-generic): `modprobe qemu_fw_cfg` →
"Module not found". So on QEMU hosts the guest never redeems its bootstrap:
no Headscale join, no rclone mount, no saved-as-of. (Firecracker uses the
kernel cmdline and is not affected.)

Fix (pick one):
- Pass the values as SMBIOS OEM strings (`-smbios type=11,path=<file>`,
  read with `dmidecode -t 11`), which the LAN test kit already does and
  which was verified in the same guest; or
- install `linux-modules-extra` matching the guest kernel in
  `provision-base.sh` and keep a meta-package so kernel updates keep it.

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

## 5. Repeated session requests create new Headscale keys (correctness, low)

When the user already has an active session, every `POST /v1/sessions` call
mints fresh Headscale preauth keys (`sessions.py:202`) instead of reusing the
existing grant. Unused keys pile up.

Fix: store the session's mesh grant and return it on repeat calls.

## 6. Repo housekeeping (low)

- Local `main` is ahead of `origin/main` (not pushed).
- `DEMO_RUNBOOK.md` contains the VPS public IP; decide whether it belongs in
  the repo.
- Three unused-import warnings in `cargo test` (`PathBuf`, `DEVICE_ARG` ×2).
- Matchmaker tests need `TEST_DATABASE_URL` (Postgres); document a one-line
  way to start a test database.

## 7. Windows setup wrongly reports virtualization off (correctness, medium)

`Assert-FirmwareVirtualization` in `host-agent/scripts/windows-host-setup.ps1`
fails when `Win32_Processor.VirtualizationFirmwareEnabled` is `False`. Windows
reports `False` whenever a hypervisor is already running (Hyper-V, WSL2,
Memory Integrity/VBS), even though VT-x/AMD-V is on. Those PCs are told to
change a BIOS setting that is already correct.

Fix: skip the check when `Win32_ComputerSystem.HypervisorPresent` is true
(the LAN test kit already does this).

## 8. No test on real hardware yet (risk, high)

Every hypervisor path (QEMU+KVM, QEMU+WHPX, VFIO, Hyper-V DDA/GPU-P) is
covered only by mocks. Nothing has shown a real desktop in a browser.
First step: the LAN test kit (`docs/runbooks/lan-test.md`). Then
`DEMO_RUNBOOK.md` and `docs/runbooks/gate-*.md`.

## Accepted for now

- The gateway skips TLS certificate checks to guests (`verify=False` in
  `gateway/src/dcl_gateway/main.py`). Acceptable because that hop travels
  inside the encrypted Headscale mesh.
