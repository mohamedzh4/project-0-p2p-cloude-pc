# Guest image pipeline

Packer + QEMU **build** of the disks Host Agent already knows how to
**fetch and verify**. Linux Standard Host Agent can `InstanceStart` Firecracker
(phase 27) when kernel + verified `rootfs.img` are present, then poll Kasm
on `https://{guestMesh.hostname}/` (HTTPS 443). Windows Standard Host Agent
can spawn QEMU `-accel whpx` (phase 28) and poll the same Kasm locator.
Linux Pro Host Agent can apply vfio-pci bind on provision (phase 29) and
poll Sunshine on `https://{guestMesh.hostname}:47990/`. Windows Pro Host
Agent can apply Hyper-V DDA/GPU-P on provision (phase 30) and poll Sunshine
the same way (VM `dcl-pro-guest` must already exist; no `Start-VM`). Metal
still needs TAP/guest net. Tests inject loopback HTTP. Timeout is 400, not
a `.invalid` success.

| Tier | Streamer in the guest | Locator after provision | Disk |
| --- | --- | --- | --- |
| Standard | [Kasm Workspaces](https://kasm.com/docs/latest/install/single_server_install.html) single-server | `https://{guestMesh.hostname}/` (HTTPS 443; Packer does not pass `-L 8443`) | `output/standard/rootfs.qcow2` + `rootfs.img` |
| Pro | [Sunshine](https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2getting__started.html) (Moonlight is the client) | `https://{guestMesh.hostname}:47990/` (web UI; GameStream base is 47989) | `output/pro/rootfs.qcow2` + `rootfs.img` |

Storage locators stay renterd S3 (not Storj). Mesh stays self-hosted Headscale
(the Tailscale *client* is installed in the guest; it is not `tailscale up`'d
at build time and this is not Tailscale SaaS).

## Minimal operator inputs

Nothing in this directory is a guest OS ISO or a private key.

1. **Packer** ≥ 1.10 and **QEMU** (`qemu-system-*` + `qemu-img`) on the build machine.
   Linux with KVM is the intended builder. macOS can use HVF (slow, fine for a disk).
   `PACKER_ACCELERATOR=none` is TCG — allowed only to *build*, never as a hosting backend.
2. **Ubuntu 24.04 cloud image** — Packer downloads the pinned URL in `versions.env`,
   or set `UBUNTU_CLOUDIMG` to a local `.img` you already have. Do not add it to git.
   A live-server ISO is **not** required for the default `disk_image` path.
3. **Dev Ed25519 PEM** — generate locally, never commit:

```bash
openssl genpkey -algorithm ED25519 -out "$HOME/dcl-dev-ed25519.pem"
export IMAGE_SIGNING_PRIVATE_KEY="$HOME/dcl-dev-ed25519.pem"
```

4. **Network** during the build so the VM can pull the Kasm tarball (checksum-pinned)
   or the Sunshine `.deb` (official `latest` Ubuntu 24.04 asset).
5. **Standard passwords** — optional. If `DCL_KASM_ADMIN_PASSWORD` /
   `DCL_KASM_USER_PASSWORD` are unset, `build.sh` generates them into
   `output/standard/kasm-credentials.txt` (gitignored).
6. **Firecracker kernel (later boot, not this build)** — Packer emits a QEMU disk
   with GRUB. Firecracker does not run GRUB. Set `FIRECRACKER_KERNEL` to a
   kernel that can mount this Ubuntu rootfs (virtio-blk/net, overlayfs, cgroup v2)
   to copy `vmlinux` next to the disk. The Firecracker getting-started demo
   kernel is not enough for Docker/Kasm. `scripts/extract_kernel.sh` will not
   guess one.

Kasm's installer needs nested Docker inside the guest (official single-server).
That is why the cloud image is grown to 64G and the Packer VM is 4 vCPU / 8 GiB
for the *build*. Host Agent InstanceStart (phase 27) still needs an operator
`FIRECRACKER_KERNEL` and does not invent a live Kasm URL. Host Agent QEMU
`-accel whpx` (phase 28) uses the verified `rootfs.img` (raw or qcow2
content) and also does not invent a live Kasm URL.

Pro GPU vendor drivers are **not** baked in (the Packer VM has no passthrough
GPU). Install NVIDIA/AMD drivers on first Pro boot after VFIO/DDA.

## Run

```bash
cd guest-images
# once
export IMAGE_SIGNING_PRIVATE_KEY="$HOME/dcl-dev-ed25519.pem"

./scripts/build.sh standard
./scripts/build.sh pro
```

`packer init` pulls the HashiCorp QEMU plugin. The build can take well over an
hour (Kasm pulls workspace images).

## Outputs (gitignored)

Under `output/<standard|pro>/`:

| File | Role |
| --- | --- |
| `rootfs.qcow2` | QEMU (Linux VFIO / Windows WHPX `-hda`; Host Agent cache name is still `rootfs.img`) |
| `rootfs.img` | raw; Host Agent cache name is `rootfs.img` |
| `rootfs.img.sha256` | GNU `hex  filename` |
| `rootfs.img.sig` | standard base64 Ed25519 over the **raw 32-byte** SHA-256 |
| `rootfs.img.manifest.json` | same shape as Matchmaker `GET /v1/images/{labTier}` plus `imageFile` |
| `env.snippet` | `IMAGE_*` lines to copy into `.env` (still do not commit `.env`) |

Signing matches Host Agent `image.rs`: SHA-256 of the file, Ed25519 over that
32-byte digest, public key 64 hex chars. Checksums and signatures sit **next
to the image**, not in git.

```bash
python3 scripts/sign_image.py checksum --image output/standard/rootfs.img
python3 scripts/sign_image.py sign --image output/standard/rootfs.img \
  --lab-tier standard   # uses IMAGE_SIGNING_PRIVATE_KEY
python3 scripts/sign_image.py public-key
```

Point the Host Agent at the bytes (HTTP or `file://`):

```
HOST_AGENT_STANDARD_IMAGE_URL=...
HOST_AGENT_PRO_IMAGE_URL=...
```

Copy digest + signature from `env.snippet` into Matchmaker `IMAGE_STANDARD_*` /
`IMAGE_PRO_*` and Host Agent `IMAGE_SIGNING_PUBLIC_KEY` + `IMAGE_LOCAL_DIGEST`
(Standard pin). Linux fetch/verify is already implemented. Windows still logs
**would fetch** (phase 20). Linux Standard can InstanceStart; set
`FIRECRACKER_KERNEL`. The provision locator stays `.invalid`.

## Tests (no hardware)

```bash
python3 -m unittest discover -s guest-images/tests -v
```

These assert template contents (Kasm vs Sunshine), checksum/signature sidecars,
and that Packer HCL does not default to TCG. They do not download Ubuntu or run
Packer.

## Honest limits

- This pipeline has **not** been run on physical hardware in this repo.
- Host Agent `POST /v1/provision` on Linux/Windows Standard may set
  `hypervisorInvoked: true` after invoke, then poll Kasm on the mesh
  hostname (443). Linux/Windows Pro GPU apply polls Sunshine (47990).
  Timeout is 400. Plan-only stubs may still return `.invalid`.
- `SEED_HOST_KASM_URL` remains `https://example.invalid/kasm` for the seed
  command. Successful provision overwrites it with the live locator.
