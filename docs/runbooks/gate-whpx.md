# GATE-WHPX — QEMU+WHPX + Kasm inside the guest (Windows host)

**Status: CLOSED (human declared PASS 2026-09-05).**
Closure is **operator-declared**, not agent-verified metal. This Mac cannot host.
Agents must not invent WHPX/QEMU logs or a live Kasm URL from this file.

Gate id: `GATE-WHPX`
Architecture: Standard Lab on Windows — QEMU with the Windows Hypervisor
Platform (WHPX) is the isolation boundary; Kasm runs *inside the guest*
(`docs/distributed-compute-lab-architecture.md` §3.1). Firecracker does not
run on Windows. Do not dual-boot Linux “to make Firecracker work” for this
gate — the point is a Windows volunteer PC.

Hyper-V as the hypervisor is an allowed fallback **only if** QEMU+WHPX cannot
be enabled and you document why. TCG (software emulation) is a **FAIL**.

## Operator declaration

On 2026-09-05 a human operator declared GATE-WHPX **PASS** and asked to skip a
second Linux/Windows host PC so the software app could be finished. They did
not paste runbook measurements. There is no `docs/phase-logs/gate-whpx-result.md`
and no live session URL in this repo. `SEED_HOST_KASM_URL` remains
`https://example.invalid/kasm`. Do not treat this closure as agent-verified
QEMU+WHPX/Kasm on metal.

## Pass / fail (read this first)

**PASS** only if all of these are true:

1. Windows 10 version 2004+ or Windows 11, x86_64 or ARM64.
2. Windows Hypervisor Platform is enabled and QEMU actually uses `-accel whpx`
   (or you documented a Hyper-V VM as the fallback).
3. The guest is Linux (Ubuntu 22.04/24.04 or Debian 12). The guest OS is
   independent of the host OS.
4. Kasm is installed **in the guest**, not on Windows.
5. A browser on the Windows host opens a **live desktop** at an `http(s)` URL
   that is not `https://example.invalid/kasm`.

**FAIL:** QEMU falls back to tcg; Kasm installed with Docker Desktop on the
host; WSL2 used as the “guest”; URL still the placeholder.

Out of scope: Matchmaker, Host Agent, Tailscale, Storj, GPU passthrough
(that is GATE-PRO). Guest egress still uses the host ISP IP.

Official references:

- QEMU WHPX: https://www.qemu.org/docs/master/system/whpx.html
- Kasm single-server: https://kasm.com/docs/latest/install/single_server_install.html

---

## 1. Enable Windows Hypervisor Platform

PowerShell **as Administrator**:

```powershell
Get-ComputerInfo | Select-Object WindowsProductName, OsHardwareAbstractionLayer
systeminfo.exe | Select-String "Hyper-V"
```

**Expect** under Hyper-V Requirements: `Virtualization Enabled In Firmware: Yes`.

```powershell
Enable-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform -All
```

Reboot if the cmdlet says so. Then:

```powershell
bcdedit /enum "{current}" | Select-String hypervisorlaunchtype
# If it is Off:
#   bcdedit /set hypervisorlaunchtype auto
#   then reboot
```

**Record:** product name, virtualization-enabled line, HypervisorPlatform
state (`Enabled`).

---

## 2. Install QEMU for Windows

1. Install a current QEMU Windows build (MSYS2 `mingw-w64-x86_64-qemu` or the
   installer from https://www.qemu.org/download/#windows).
2. Confirm the binary:

```powershell
qemu-system-x86_64.exe --version
qemu-system-x86_64.exe -accel help
```

**Expect:** `whpx` listed among accelerators.

---

## 3. Create a guest disk and download Ubuntu

```powershell
New-Item -ItemType Directory -Force -Path C:\dcl-gate-whpx | Out-Null
Set-Location C:\dcl-gate-whpx
qemu-img.exe create -f qcow2 kasm-guest.qcow2 64G
```

Download an Ubuntu 24.04 live-server or desktop ISO from ubuntu.com into
`C:\dcl-gate-whpx\ubuntu.iso`. Desktop is easier for a first Kasm; server is
fine if you are comfortable with SSH only.

**Expect:** `qemu-img.exe info kasm-guest.qcow2` shows 64 GiB.

---

## 4. Boot with WHPX (not tcg)

```powershell
qemu-system-x86_64.exe -accel whpx -M q35 `
  -smp 4 -m 8G `
  -device qemu-xhci -device usb-tablet `
  -drive file=C:\dcl-gate-whpx\kasm-guest.qcow2,if=virtio,index=0 `
  -cdrom C:\dcl-gate-whpx\ubuntu.iso -boot d `
  -nic user,model=virtio,hostfwd=tcp::8443-:443 `
  -vga virtio -display gtk
```

On ARM64 Windows use `qemu-system-aarch64.exe` with a matching ARM Ubuntu ISO
and `-accel whpx` (see the same QEMU WHPX page).

**Expect:**

- QEMU window opens. Installer runs at usable speed (not like a 1990s emulator).
- stderr must **not** contain `failed to initialize WHPX` or `No accelerator found`.
- If WHPX init fails, this gate is **FAIL**. Do not silently add `-accel tcg`.

Install Ubuntu onto the qcow2. After install, drop `-cdrom` / `-boot d` and
boot from the disk. Enable OpenSSH if you used the server ISO.

**Record:** the exact QEMU command line and whether WHPX initialized.

---

## 5. Install Kasm inside the guest

In the guest (terminal or SSH via the forwarded port if you add `hostfwd` for
22):

```bash
hostnamectl
# OS should be Ubuntu/Debian, not Windows.

cd /tmp
curl -O https://kasm-static-content.s3.amazonaws.com/kasm_release_1.17.0.7f020d.tar.gz
tar -xf kasm_release_*.tar.gz
sudo bash kasm_release/install.sh
```

Use the tarball name from the current Kasm docs if the filename above moved.

**Expect:** installer finishes; Kasm listens on 443 in the guest.

The QEMU line above forwards host `8443` → guest `443`. On the Windows host:

```powershell
curl.exe -kI https://127.0.0.1:8443/
```

**Expect:** HTTP 200 or 302.

---

## 6. Open the desktop from a browser

On the Windows host, open `https://127.0.0.1:8443/`, accept the self-signed
certificate, log in with the passwords Kasm printed, launch a workspace.

**Expect:** interactive desktop. Copy that URL — it later becomes
`SEED_HOST_KASM_URL` (for a LAN demo you may instead use the guest's
user-net or TAP IP; write down whichever URL the browser actually used).

---

## 7. Hyper-V fallback (only if step 4 cannot use WHPX)

If `-accel whpx` cannot be made to work after enabling HypervisorPlatform and
`hypervisorlaunchtype auto`:

1. Write the WHPX error into the result file.
2. Create a Generation 2 Hyper-V VM, 4 vCPU, 8 GiB, 64 GiB disk, Ubuntu guest.
3. Install Kasm in that guest and browse to its URL from the host.
4. Mark the result as **Hyper-V fallback**, not QEMU+WHPX. That can still
   pass GATE-WHPX because the architecture allows Hyper-V when WHPX is
   unavailable — but say so explicitly.

WSL2 is **not** an acceptable fallback (shared kernel with the host).

---

## 8. Report results

Create **`docs/phase-logs/gate-whpx-result.md`** with this shape. The operator
closed GATE-WHPX **without** attaching that file; the filled template below is
the in-runbook record only.

```markdown
# GATE-WHPX result

- Date (ISO): 2026-09-05
- Operator (human name): operator declared PASS; measurements not attached
- Host Windows edition / build / arch: operator declared PASS; measurements not attached
- HypervisorPlatform enabled: operator declared PASS; measurements not attached
- Accelerator actually used: operator declared PASS; measurements not attached
- QEMU version (or Hyper-V VM generation): operator declared PASS; measurements not attached
- Guest OS: operator declared PASS; measurements not attached
- Guest vCPU / RAM / disk: operator declared PASS; measurements not attached
- Live URL: operator declared PASS; measurements not attached
- `curl -kI` status line: operator declared PASS; measurements not attached
- WHPX errors (paste if any): operator declared PASS; measurements not attached
- Failures / workarounds: operator declared PASS; measurements not attached
- Human confirms GATE-WHPX may be marked closed: yes (operator declaration 2026-09-05)
```

Do not commit Kasm passwords. Do not invent lab output to replace the lines
above. `SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`. There is
still no live QEMU+WHPX/Kasm session in this repo.
