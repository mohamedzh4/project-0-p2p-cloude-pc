# Standard Linux (QEMU+KVM)

Implemented in code. Not yet run on physical hardware through the Host
Agent provision path. (The LAN test kit, which uses the same QEMU flags,
showed a desktop on an Arch PC on 2026-11-14; see `docs/test-logs/`.)

See `PROJECT_STATUS.md`. Guest disks: `guest-images/README.md`.
Never write "confirmed working" until a human reports a live session on metal.
Do not invent lab output.

Host Agent `POST /v1/provision` starts the distro `qemu-system-x86_64` with
`-enable-kvm -M q35 -cpu host`, the verified disk on virtio-blk, and QEMU
SLIRP user networking (`qemu_kvm.rs`). The guest boots the kernel inside its
own disk. After the spawn, provision polls KasmVNC on
`https://{guestMesh.hostname}:8443/` and returns that URL, or 400 if it never
answers. Tests inject a fake process and a loopback streamer.

## Operator requirements (metal)

Unattended blockers. Meeting them does not close the hardware line.

1. Linux x86-64 host with KVM loaded and `/dev/kvm` readable/writable by
   the agent's user, so the capability probe offers Standard.
2. Distro QEMU installed (`qemu-system-x86`; the agent looks for
   `/usr/bin/qemu-system-x86_64`, or `HOST_AGENT_QEMU_BIN`). Nothing is
   downloaded.
3. At least 2 spare vCPU, 4 GiB RAM, and 50 GiB free disk on the image
   cache volume (Kasm single-server minimum; `eligibility.rs`).
4. Verified Standard `rootfs.img` (Packer
   `guest-images/output/standard/rootfs.img`, fetched from
   `HOST_AGENT_STANDARD_IMAGE_URL` and checked against the Matchmaker
   `IMAGE_STANDARD_*` digest and signature).
5. Guest network is SLIRP (no TAP, no root). The guest reaches the mesh with
   its own outbound Headscale connection; the Matchmaker origin and
   bootstrap token reach it as SMBIOS OEM strings.
6. Matchmaker session end / silence POSTs `/v1/teardown` (host bootstrap
   Bearer) to kill QEMU and clear occupancy.
