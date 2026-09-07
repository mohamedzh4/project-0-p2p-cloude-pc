# Pro GPU (VFIO / Hyper-V DDA or GPU-P + Sunshine)

Implemented in code. Not yet run on physical hardware.

See `PROJECT_STATUS.md`. Guest disks: `guest-images/README.md`.
Do not invent lab output. Never write "confirmed working" until a human
reports a live session on metal.

## Window vs provision

The Host Agent **settings window is plan-only** (`apply: false`). Opening it
must not unbind a GPU. Linux Pro **vfio-pci apply** runs when Matchmaker
POSTs `POST /v1/provision` and the capability probe offered Pro (IOMMU +
second display path). Windows Hyper-V DDA/GPU-P apply from that provision
path is a later phase; the window still does not spawn PowerShell.

This phase does **not** attach the bound GPU to a QEMU boot. Locators stay
`https://provision.invalid/qemu-vfio`. `SEED_HOST_KASM_URL` stays
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

## Metal leftovers (not confirmed working)

- IOMMU enabled, `vfio-pci` loaded, GPU in an IOMMU group, second display
  path so the host keeps a screen.
- Windows: Hyper-V feature, elevated agent, existing VM `dcl-pro-guest`
  (provision apply still later). Documented DDA cmdlets remain
  `Disable-PnpDevice`, `Dismount-VMHostAssignableDevice -Force`,
  `Add-VMAssignableDevice` (GPU-P only when exclusive `PartitionCount 1`).
- A later phase still needs QEMU VFIO attach, Sunshine locators, and TAP/net.
