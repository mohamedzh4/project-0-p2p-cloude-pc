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

This phase does **not** attach the bound GPU to a QEMU boot and does
**not** `Start-VM`. Locators stay `https://provision.invalid/qemu-vfio`
(Linux) and `https://provision.invalid/hyperv-gpu` (Windows). Sunshine
stays `https://provision.invalid/sunshine`. `SEED_HOST_KASM_URL` stays
`https://example.invalid/kasm`.

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

No `Start-VM`. Default VM name `dcl-pro-guest` must already exist and be
powered off.

## Metal leftovers (not confirmed working)

- IOMMU enabled, `vfio-pci` loaded, GPU in an IOMMU group, second display
  path so the host keeps a screen.
- Windows: Hyper-V feature, elevated agent (UAC; `Disable-PnpDevice` /
  `Dismount-VMHostAssignableDevice` need admin; this agent does not
  auto-elevate), existing powered-off VM `dcl-pro-guest`.
- A later phase still needs QEMU VFIO attach, live Sunshine locators, and TAP/net.
