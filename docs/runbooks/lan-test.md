# LAN hardware test

The smallest real-hardware proof: a friend boots the signed Standard guest on
their PC, and someone on the same Wi-Fi opens the desktop in a browser. No
VPS, Matchmaker, gateway, Headscale or renterd is involved, so a failure here
is the hypervisor, the guest image or KasmVNC — nothing else.

```
second device browser --LAN--> host PC :8443 --QEMU SLIRP hostfwd--> guest KasmVNC :8443
```

Kit sources: `host-agent/scripts/lan-test/`.

## What it proves and what it does not

| Proves | Does not prove |
| --- | --- |
| KVM (Linux) / WHPX (Windows) works on that PC | Host Agent app, provisioning HTTP, Matchmaker |
| The signed guest boots with the same QEMU flags as the Host Agent | Headscale mesh, gateway token, public HTTPS |
| XFCE + KasmVNC render in a real browser | rclone persistent storage |
| Boot time on real hardware (in the report) | Pro GPU (VFIO / Hyper-V) |

After it passes, run the full path in `DEMO_RUNBOOK.md`.

## 1. Build the guest image (once)

On a Linux machine with `/dev/kvm` (your Mac cannot do this):

```bash
cd guest-images
export IMAGE_SIGNING_PRIVATE_KEY="$HOME/dcl-image-signing.pem"
./scripts/build.sh standard
```

The image must be built **after** the LAN password change in
`guest-images/scripts/provision-kasm.sh`. An older image still boots, but the
desktop opens with no password; the script then prints a warning.

## 2. Pack the kits

```bash
host-agent/scripts/lan-test/pack-lan-test-kit.sh
```

This reads the digest from `guest-images/output/standard/env.snippet` and
writes `host-agent/dist-lan-test/dcl-lan-test-{linux,windows}.zip`. Options:

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
with SHA-512 check), verifies the image SHA-256, boots it with `-snapshot`
(the signed disk is never modified), and prints:

```
https://<lan-ip>:8443   user: ubuntu   password: <random 12 chars>
```

The port is opened to the local subnet only and closed again on exit.

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
- A VPN on the host can make the script pick the VPN address; turn it off.
- Some routers isolate Wi-Fi clients ("AP isolation" / guest network). Then
  the second device cannot reach the host even when the test passes locally.
