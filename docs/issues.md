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

## 5. Repeated session requests create new Headscale keys (correctness, low)

When the user already has an active session, every `POST /v1/sessions` call
mints fresh Headscale preauth keys (`sessions.py:202`) instead of reusing the
existing grant. Unused keys pile up.

Fix: store the session's mesh grant and return it on repeat calls.

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
benchmarks need one typed word. The signed Packer image does not have
these yet.

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

## Accepted for now

- The gateway skips TLS certificate checks to guests (`verify=False` in
  `gateway/src/dcl_gateway/main.py`). Acceptable because that hop travels
  inside the encrypted Headscale mesh.
