# GATE-PRO — exclusive GPU assignment + Sunshine/Moonlight

**Status: OPEN until a human fills the result template on real hardware.**
This Mac cannot host. Agents must not mark GATE-PRO closed from this file alone.

Gate id: `GATE-PRO`
Architecture: Pro/Gamer Lab — the guest gets a **whole GPU** for the session.
Firecracker has **no** PCI passthrough; do not try to pass a GPU into
Firecracker (`docs/distributed-compute-lab-architecture.md` §3.2 and “Why the
split is mandatory”).

| Host OS | Hypervisor | Streaming |
| --- | --- | --- |
| Linux | QEMU/KVM + VFIO | Sunshine in the guest, Moonlight on a client |
| Windows | Hyper-V DDA or GPU-P, or QEMU+WHPX passthrough if the hardware allows it | same |

CPU/RAM sharing is concurrent. **GPU sharing is not.** The Host Agent already
says so in the picker copy. This gate exists to prove that sentence on metal.

## Pass / fail (read this first)

**PASS** only if all of these are true:

1. You recorded the PCI / instance id of the dedicated GPU (`sharedGpuId` in
   Host Agent terms — e.g. `0000:01:00.0` or the Windows instance path).
2. For the session, that GPU is bound to the guest (VFIO / DDA / GPU-P).
3. The **host** cannot use that GPU during the session: no host display on it,
   `nvidia-smi` / Task Manager / `lspci` on the host shows it unavailable to
   the host driver (vfio-pci, or not present to the host WDDM stack).
4. The host still has a display path (iGPU or a second card). Single-GPU
   “hide the host desktop” hacks are not a pass — we need the volunteer to
   keep using the PC.
5. Sunshine runs **inside the guest**. Moonlight (or another GameStream
   client) on a second machine (or the host’s remaining GPU) shows a
   playable stream. Note the round-trip feel; sub-50 ms is a target, not a
   hard fail if you record the number.

**FAIL:** GPU remains on the host driver; Sunshine installed on the host
instead of the guest; Firecracker used; SR-IOV/vGPU; “it worked but the host
went black and stayed black” without a second display path.

Out of scope: Matchmaker auto-provision, Tailscale, billing, anti-cheat titles.
Consumer GPU EULAs still need legal review before commercial launch
(architecture §9 item 10).

Official starting points:

- IOMMU / VFIO: Arch wiki PCI passthrough
  (https://wiki.archlinux.org/title/PCI_passthrough_via_OVMF)
- Sunshine: https://docs.lizardbyte.dev/projects/sunshine/latest/
- Moonlight: https://moonlight-stream.org/

Pick **one** host OS and follow that section. Do not mix commands.

---

## A. Linux host — VFIO

### A1. Firmware and IOMMU

1. In UEFI, enable Intel VT-d or AMD-Vi / IOMMU. Enable virtualization (VT-x / SVM).
2. Keep the host desktop on iGPU or GPU0. The card you will pass through should
   not be the host’s only display.

```bash
# Intel
sudo dmesg | grep -i -e DMAR -e IOMMU
# AMD
sudo dmesg | grep -i -e AMD-Vi -e IOMMU
```

**Expect:** IOMMU enabled, not “No ACPI DMAR table”.

Add the matching kernel parameter, then `sudo update-grub` and reboot:

- Intel: `intel_iommu=on iommu=pt`
- AMD: `amd_iommu=on iommu=pt`

### A2. Identify the GPU and its IOMMU group

```bash
lspci -nn | grep -E "VGA|3D|Audio"
```

Note vendor:device ids for the GPU **and** its audio function (they must be
passed together), e.g. `10de:xxxx` and `10de:yyyy`.

```bash
shopt -s nullglob
for d in /sys/kernel/iommu_groups/*/devices/*; do
  n=${d#*/iommu_groups/*}; n=${n%%/*}
  printf 'IOMMU Group %s ' "$n"
  lspci -nns "${d##*/}"
done | sort -V
```

**Expect:** the GPU and its HDMI/DP audio are alone in a group, or only with
their own functions. If the group contains a USB controller or the host’s
boot disk, **stop** — ACS override is a security trade-off and is not a pass
for this gate unless you document it and the human still wants to proceed.

### A3. Bind the dedicated GPU to vfio-pci

Example (replace ids):

```bash
echo 'options vfio-pci ids=10de:xxxx,10de:yyyy' | sudo tee /etc/modprobe.d/vfio.conf
```

Load `vfio`, `vfio_iommu_type1`, `vfio_pci` early (distro-specific: modules-load
or dracut/initramfs). Reboot.

```bash
lspci -nnk | grep -A4 -E "VGA|3D"
```

**Expect** on the dedicated card: `Kernel driver in use: vfio-pci`.
**Expect** on the host’s remaining GPU/iGPU: the normal vendor driver
(`i915`, `amdgpu`, `nvidia`).

On the host, `nvidia-smi` (if NVIDIA) must **not** list the dedicated card
while it is bound to vfio-pci.

**Record:** PCI BDF (`0000:01:00.0` form) — this is the `sharedGpuId` you
would later store. Host still has a desktop on the other GPU.

### A4. Boot a QEMU/KVM guest with that GPU

Use libvirt or a QEMU command that adds the vfio-pci device, UEFI (OVMF), and
enough RAM. Guest OS: Windows (broadest app support) or Linux + Proton.

```bash
# example shape — fill paths and the BDF from A3
qemu-system-x86_64 -enable-kvm -M q35 -cpu host -smp 8 -m 16G \
  -bios /usr/share/OVMF/OVMF_CODE.fd \
  -drive file=pro-guest.qcow2,if=virtio \
  -device vfio-pci,host=0000:01:00.0 \
  -device vfio-pci,host=0000:01:00.1
```

**Expect:** guest installs the GPU vendor driver and sees the card. Host
display stays up on the other GPU.

### A5. Sunshine in the guest, Moonlight on a client

Inside the guest, install Sunshine from
https://github.com/LizardByte/Sunshine/releases (or the distro package).
Start it. PIN a Moonlight client on another machine (phone, laptop, or the
host using the remaining GPU).

**Expect:** Moonlight connects; a 3D app or the desktop streams. Write down
Moonlight’s displayed latency or a stopwatch estimate of input-to-photon.

Stop the VM when done. Confirm the host can take the GPU back (unbind vfio or
reboot) if you want; for the gate, the important screenshot is **during** the
session: host missing the GPU, guest using it.

---

## B. Windows host — Hyper-V DDA or GPU-P

### B1. Hardware

- Windows 10/11 Pro or Enterprise (Hyper-V).
- A GPU Hyper-V can assign, **plus** iGPU or a second card for the host
  desktop.
- Firmware: virtualization and IOMMU/VT-d / AMD-Vi on.

```powershell
Get-VMHost | Format-List
systeminfo.exe | Select-String "Hyper-V"
```

### B2. Identify the GPU

```powershell
Get-PnpDevice -Class Display | Format-Table Status, Class, FriendlyName, InstanceId -AutoSize
```

**Record** the `InstanceId` of the card you will dedicate (this is
`sharedGpuId` on Windows). Confirm another display adapter remains for the host.

### B3. Assign the GPU to a VM

Follow current Microsoft docs for your path:

- Discrete Device Assignment (DDA) — whole-device, closest to VFIO.
- GPU-P / GPU Partitioning — only if you assign the **entire** usable
  partition to the guest for the session. A shared slice that the host still
  renders with is **not** exclusivity; do not call that a pass.

High-level DDA shape (names are examples):

```powershell
# Disable the device on the host, then assign to the VM.
# Use the cmdlets/docs for your Windows build (Dismount-VMHostAssignableDevice /
# Add-VMAssignableDevice, or the GPU-P Enable-VMGpuPartitionAdapter path).
Get-VM -Name "pro-guest"
```

**Expect:** host Device Manager shows the dedicated GPU gone or “hidden” from
the host; the guest Device Manager shows it. The host desktop remains on the
other adapter. If the host screen goes black and stays black, that is a
**FAIL** for “hosts keep using the PC.”

### B4. Sunshine / Moonlight

Install Sunshine in the **guest**, Moonlight on a client. Same pass criteria
as A5.

QEMU+WHPX with passthrough is allowed only if you can show the GPU is assigned
to that QEMU process and gone from the host, with the same exclusivity checks.

---

## C. Report results

Create **`docs/phase-logs/gate-pro-result.md`**:

```markdown
# GATE-PRO result

- Date (ISO):
- Operator (human name):
- Host OS / kernel or Windows build:
- Path used: Linux VFIO / Hyper-V DDA / Hyper-V GPU-P / QEMU+WHPX passthrough
- Dedicated GPU model:
- sharedGpuId (PCI BDF or Windows InstanceId):
- Host remaining display path (iGPU / second GPU):
- Evidence host lost the GPU during the session (command output or Device Manager note):
- Evidence guest gained the GPU (driver + device name):
- Sunshine version (in the guest):
- Moonlight client OS:
- Observed latency (ms) or qualitative note:
- Live stream worked: yes / no
- Failures / workarounds:
- Human confirms GATE-PRO may be marked closed: yes / no
```

Do not commit Sunshine credentials or host serials you do not need. A later
agent may update `PROJECT_STATUS.md` only when this result file exists **and**
a human explicitly says to close GATE-PRO.

Until then the gate stays **OPEN**. Do not treat a Host Agent GPU picker
screenshot as a substitute for this runbook.
