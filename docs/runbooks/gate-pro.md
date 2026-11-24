# Pro GPU (VFIO / Hyper-V DDA or GPU-P + Sunshine)

Implemented in code. Not yet run on physical hardware.

See `PROJECT_STATUS.md`. Guest disks: `guest-images/README.md`.
Do not invent lab output. Never write "confirmed working" until a human
reports a live session on metal.

## Window vs provision

The Host Agent **settings window is plan-only** (`apply: false`,
`hyperv: None`). Opening it must not unbind a GPU or spawn PowerShell.
Linux Pro **vfio-pci apply** and Windows Pro **Hyper-V DDA/GPU-P apply**
run when Matchmaker POSTs `POST /v1/provision` and the capability probe
offered Pro.

This phase **attaches** the bound GPU to a QEMU+KVM boot when distro QEMU
is installed, and **does** `Start-VM` on Windows Pro after DDA/GPU-P.
After GPU apply, provision polls Sunshine on
`https://{guestMesh.hostname}:47990/` (web UI; GameStream base is 47989)
and returns that URL, or 400 if it never answers. Tests inject loopback
HTTP. Metal still needs a guest that is actually serving Sunshine.
`SEED_HOST_KASM_URL` stays `https://example.invalid/kasm` for the seed
command.

## Linux vfio-pci sysfs contract

Kernel vfio.rst + PCI `new_id` (hex `vendor device`). Sequence the agent
writes, with rollback to the previous driver if a mid-step fails:

1. `driver_override` ← `vfio-pci` on `/sys/bus/pci/devices/<BDF>/`
2. Host driver `unbind` ← `<BDF>` (`DDDD:BB:DD.F`)
3. `/sys/bus/pci/drivers/vfio-pci/new_id` ← `<vendor> <device>`
4. `/sys/bus/pci/drivers/vfio-pci/bind` ← `<BDF>`

PCI addresses come from sysfs (not `lspci`). GPU assignment is exclusive
for the session (`exclusive: true`). Tests inject a mock tree and refuse
live `/sys` under `cfg(test)`.

## Windows Hyper-V DDA / GPU-P contract

Documented Microsoft cmdlets. DDA is preferred when a location path exists.
GPU-P is exclusive only (`PartitionCount 1`, full 100000000 resource
fractions). Tests inject `HyperVGpuOps` and refuse live `powershell.exe`
under `cfg(test)`. Production is `powershell.exe -NoProfile -NonInteractive`
with a 30s kill.

DDA:

1. `Set-VM` (AutomaticStopAction TurnOff, GuestControlledCacheTypes,
   Low/HighMemoryMappedIoSpace) — not rolled back
2. `Disable-PnpDevice -Confirm:$false`
3. `Dismount-VMHostAssignableDevice -Force -LocationPath`
4. `Add-VMAssignableDevice -LocationPath -VMName`
5. `Get-VMAssignableDevice` must confirm; empty output rolls back

DDA rollback: `Remove-VMAssignableDevice` → `Mount-VMHostAssignableDevice`
→ `Enable-PnpDevice -Confirm:$false`.

GPU-P (only when DDA has no location path and partition count 1 is valid):

1. `Set-VMHostPartitionableGpu -PartitionCount 1`
2. `Add-VMGpuPartitionAdapter -VMName -InstancePath`
3. `Set-VMGpuPartitionAdapter` min/max/optimal 100000000
4. `Get-VMGpuPartitionAdapter` must confirm

GPU-P rollback: `Remove-VMGpuPartitionAdapter` → restore partition count.

After a successful DDA or GPU-P verify, provision runs `Start-VM`.
Default VM name `dcl-pro-guest` must already exist. Teardown is
`Stop-VM -Force -TurnOff` then GPU rollback.

Linux Pro QEMU attach (phase 34) uses distro `qemu-system-*` with
`-device vfio-pci,host=<BDF>` and SLIRP user-net. Set `HOST_AGENT_QEMU_BIN`
or install the package (free). Weilnetz NSIS is Windows-only.

## Metal leftovers (not confirmed working)

- IOMMU enabled, `vfio-pci` loaded, GPU in an IOMMU group, second display
  path so the host keeps a screen.
- Windows: Hyper-V feature, elevated agent (UAC; `Disable-PnpDevice` /
  `Dismount-VMHostAssignableDevice` need admin; this agent does not
  auto-elevate), existing VM `dcl-pro-guest`.
- Guest join on Hyper-V has no fw_cfg; the baked unit no-ops without
  `dcl.matchmaker` / `dcl.bootstrap`. Live Sunshine on metal is still a
  human report.

## First Pro test: Arch PC and Windows PC

Both test PCs have a GPU. Do the steps in order and stop at the first
failure; record each run in `docs/test-logs/`.

### Step 1: readiness check (read-only, 1 minute per PC)

- **Arch:** `bash host-agent/scripts/pro-gpu-check.sh`
- **Windows** (admin PowerShell):
  `powershell -ExecutionPolicy Bypass -File host-agent\scripts\pro-gpu-check.ps1`

Each writes `pro-gpu-check-<pc>-<time>.txt`. Bring both reports back. They
answer: which GPU can be given away, does the PC keep a screen, is IOMMU /
Hyper-V on, and what the agent will see (issue 35).

### Step 2: host setup, depending on the reports

Arch:

1. BIOS: VT-d (Intel) or SVM + IOMMU (AMD) on. If the CPU has an iGPU, turn
   it on and plug the monitor into the motherboard, so the card is free.
2. Kernel command line: add `intel_iommu=on iommu=pt` (Intel) or
   `iommu=pt` (AMD), rebuild the bootloader config, reboot.
3. `sudo modprobe vfio-pci` and add `vfio-pci` to
   `/etc/modules-load.d/vfio.conf`.
4. `sudo pacman -S qemu-full edk2-ovmf`.
5. Re-run the check: the card should show "good candidate for Pro".

Windows:

1. Windows Pro/Enterprise/Education with Hyper-V enabled.
2. iGPU on in the BIOS, monitor on the motherboard.
3. Create a Generation 2 VM named `dcl-pro-guest` with the Pro disk
   attached (Secure Boot off for the Linux guest).
4. Re-run the check as admin. Note whether the GPU lists partition count 1
   (needed for GPU-P, issue 36).

### Step 3: GPU hand-over by hand, no platform

Prove the hardware before the agent: give the card to a throwaway guest by
hand and see the guest's `lspci` / `nvidia-smi` list it, then give it back
and check the host's screen and driver recover. Linux: the four sysfs
writes in "Linux vfio-pci sysfs contract" above, then
`qemu-system-x86_64 -enable-kvm -machine q35 -m 4G -device vfio-pci,host=<BDF> ...`.
Windows: the DDA steps above on `dcl-pro-guest`; if `Dismount-VMHostAssignableDevice`
fails, try GPU-P.

### Step 4: Pro image and server

1. Build and sign the Pro disk on the Arch PC: `guest-images/scripts/build.sh pro`.
2. Upload it and set `IMAGE_PRO_*` on the server, `HOST_AGENT_PRO_IMAGE_URL`
   on the hosts.

### Step 5: full booking

With an agent built after the issue 35 fix: book a Pro
session from the web page, connect with Moonlight over the mesh, then end
the session and check the host gets its GPU back.
