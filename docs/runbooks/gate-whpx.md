# Standard Windows (QEMU+WHPX)

Implemented in code. Not yet run on physical hardware.

See `PROJECT_STATUS.md`. Guest disks: `guest-images/README.md`.
Never write "confirmed working" until a human reports a live session on metal.
Do not invent lab output. After a successful spawn, provision polls Kasm
on `https://{guestMesh.hostname}/` (HTTPS 443) and returns that URL, or 400
if it never answers. Tests inject loopback HTTP. Metal still needs guest net.

Phase 28 starts QEMU (`-accel whpx`) from Host Agent `POST /v1/provision`
when Windows Standard is offered. Phase 31 polls Kasm after that invoke.
That is a software contract. It is not a live Kasm session on metal. Hyper-V
is the Pro GPU backend, not a Standard boot fallback. TCG is not the
Standard Windows product.

## Operator requirements (metal)

Unattended blockers. Meeting them does not close the hardware line.

1. Windows host with Windows Hypervisor Platform enabled so the capability
   probe offers Standard (`DISM /online /Enable-Feature
   /FeatureName:HypervisorPlatform /All`).
2. Phase-24 Weilnetz QEMU already in the user hypervisor cache (NSIS `/S`
   `/D=`; UAC consent is still manual).
3. Verified Standard disk (phase 20 fetch / phase 26 Packer
   `guest-images/output/standard/rootfs.img`). Host Agent cache name is
   `rootfs.img`; `-hda` accepts raw or qcow2 content.
4. Guest net is QEMU SLIRP user-net (no TAP-Windows). A successful spawn
   plus locator poll still needs a reachable Kasm on the mesh hostname.
   Timeout is 400, not a `.invalid` success.
5. Matchmaker session end/silence POSTs `/v1/teardown` (host bootstrap
   Bearer) to kill QEMU and clear occupancy.
