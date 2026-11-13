# LAN hardware test

The smallest real-hardware proof: a friend boots the Standard guest on their
PC, and someone on the same Wi-Fi opens the desktop in a browser. No VPS,
Matchmaker, gateway, Headscale or renterd is involved, so a failure here is
the hypervisor, the guest or KasmVNC — nothing else.

```
second device browser --LAN--> host PC :8443 --QEMU SLIRP hostfwd--> guest KasmVNC :8443
```

Kit sources: `host-agent/scripts/lan-test/`.

## Two modes

| | **quick** (default) | **signed** |
| --- | --- | --- |
| Needs an image build | no | yes (`build.sh standard` on Linux+KVM) |
| Guest disk | official Ubuntu 24.04 cloud image, pinned dated release + SHA-256 | your Packer image, SHA-256 from `env.snippet` |
| Desktop install | first boot, via cloud-init running the same `provision-kasm.sh` | baked in |
| First run | 15–45 min (downloads ~600 MB + ~500 MB packages) | ~1–5 min |
| Later runs | ~1–5 min (install kept in an overlay disk) | ~1–5 min (`-snapshot`, image never changes) |
| Friend's PC needs internet | yes | only if the disk is downloaded |

Quick mode proves the hardware path today. Signed mode additionally proves
your own built image; run it once you have a Linux build machine.

## What it proves and what it does not

| Proves | Does not prove |
| --- | --- |
| KVM (Linux) / WHPX (Windows) works on that PC | Host Agent app, provisioning HTTP, Matchmaker |
| The guest boots with the same QEMU flags as the Host Agent | Headscale mesh, gateway token, public HTTPS |
| XFCE + KasmVNC render in a real browser | rclone persistent storage |
| Boot time on real hardware (in the report) | Pro GPU (VFIO / Hyper-V) |

After it passes, run the full path in `DEMO_RUNBOOK.md`.

## 1. Pack the kits

Quick mode (no image needed):

```bash
host-agent/scripts/lan-test/pack-lan-test-kit.sh
```

This writes `host-agent/dist-lan-test/dcl-lan-test-{linux,windows}.zip`
with the scripts and a `seed.iso` (cloud-init user-data that embeds
`guest-images/scripts/provision-kasm.sh` and the pinned KasmVNC package from
`guest-images/versions.env`). Rebuild the kits whenever `provision-kasm.sh`
changes; the friend's PC redoes the desktop install when `seed.iso` changes.

Signed mode, after `guest-images/scripts/build.sh standard` on a Linux KVM
machine:

```bash
host-agent/scripts/lan-test/pack-lan-test-kit.sh --signed
```

- `--image guest-images/output/standard/rootfs.qcow2` copies the disk into
  both kits (large).
- `--qcow2-url https://…/rootfs.qcow2` makes the kit download the disk.
- `--digest <sha256>` if you have no local build output.

Otherwise send `rootfs.qcow2` separately; the friend puts it next to the
script.

## 3. Friend runs it

Instructions are in the kit's `README.txt`.

- Windows: double-click `lan-test.cmd`, accept the admin prompt. If WHPX was
  off, the script turns it on and asks for a reboot; run it again after.
- Ubuntu: `bash lan-test.sh`.

The script checks the PC, installs QEMU (Windows: the pinned Weilnetz build
with SHA-512 check), verifies the image SHA-256, boots it, and prints:

```
https://<lan-ip>:8443   user: ubuntu   password: <random 12 chars>
```

The port is opened to the local subnet only and closed again on exit. If the
desktop answers without asking for a password (an image older than the LAN
password change), the script stops the VM instead of exposing it.

## 4. Read the report

The friend sends back `lan-test-report-<time>.txt`: OS, CPU, RAM, checks,
boot time and `Result: PASS/FAIL`. It never contains the password. On
failure it includes the QEMU error output or the last guest console lines.

Only check a PC without booting: `bash lan-test.sh --check` or
`lan-test.cmd -CheckOnly`.

## Checking the kit without hardware

```bash
host-agent/scripts/lan-test/e2e/run-docker-e2e.sh
```

Quick mode with a real Ubuntu VM under software emulation (slow, about an
hour on an Apple-silicon Mac; see `e2e/quick-tcg.sh`):

```bash
host-agent/scripts/lan-test/e2e/run-quick-tcg.sh
```

Needs Docker only (works on a Mac). It runs `lan-test.sh` end to end in an
x86-64 Ubuntu container against a fake QEMU (password, LAN link, digest
refusal, crash reporting, cleanup), then runs the guest
KasmVNC password helpers against the real pinned KasmVNC package in both
modes (LAN password on, mesh/gateway mode off). It does not boot a real VM
and does not cover `lan-test.ps1` beyond a syntax parse.

## Known limits

- x86-64 only. Windows on ARM is not covered.
- The image digest is checked; the Ed25519 signature is not (the digest and
  the kit come from the same sender, so it would add nothing here).
- Quick mode depends on Ubuntu keeping the dated cloud-image folder online
  (they are kept for months, not forever). Update the URL/SHA-256 in both
  `lan-test.sh` and `lan-test.ps1` when it disappears.
- A VPN on the host can make the script pick the VPN address; turn it off.
- Some routers isolate Wi-Fi clients ("AP isolation" / guest network). Then
  the second device cannot reach the host even when the test passes locally.
