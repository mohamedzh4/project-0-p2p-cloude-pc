# Standard Linux (Firecracker)

Implemented in code. Not yet run on physical hardware.

See `PROJECT_STATUS.md`. Guest disks: `guest-images/README.md`.
Never write "confirmed working" until a human reports a live session on metal.
Do not invent lab output. After a successful invoke, provision polls Kasm
on `https://{guestMesh.hostname}/` (HTTPS 443) and returns that URL, or 400
if it never answers. Tests inject loopback HTTP. Metal still needs TAP/net.

Phase 27 starts Firecracker (`InstanceStart`) from Host Agent
`POST /v1/provision` when Linux Standard is offered. Phase 31 polls Kasm
after that invoke. That is a software contract. It is not a live Kasm
session on metal.

## Operator requirements (metal)

Unattended blockers. Meeting them does not close the hardware line.

1. Linux host with KVM loaded and `/dev/kvm` readable/writable so the
   capability probe offers Standard.
2. Phase-19 Firecracker v1.16.1 already in the user hypervisor cache
   (SHA-256 verified). This runbook does not fetch it.
3. Verified Standard `rootfs.img` (phase 20 fetch / phase 26 Packer
   `guest-images/output/standard/rootfs.img`).
4. Operator kernel via `FIRECRACKER_KERNEL` (or `vmlinux` next to the
   rootfs). Packer does not invent a kernel from GRUB. The Firecracker
   getting-started demo kernel is not enough for Docker/Kasm.
5. TAP/guest network is **not** in this runbook. InstanceStart plus locator
   poll still needs a reachable Kasm on the mesh hostname. Timeout is 400,
   not a `.invalid` success.
6. Jailer is unused. Teardown HTTP is unused (occupancy is not cleared
   when Matchmaker interrupts).
