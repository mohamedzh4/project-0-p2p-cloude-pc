# GATE-FC — Firecracker + Kasm inside the guest (Linux host)

**Status: CLOSED (human declared PASS 2026-09-05).**
Closure is **operator-declared**, not agent-verified metal. This Mac cannot host.
Agents must not invent Firecracker boot logs, KVM checks, or a live Kasm URL
from this file.

Gate id: `GATE-FC`
Architecture: Standard Lab on Linux — Firecracker microVM is the isolation
boundary; Kasm runs *inside the guest*, not as Docker on the host kernel
(`docs/distributed-compute-lab-architecture.md` §3.1 and §9 item 1).

## Operator declaration

On 2026-09-05 a human operator declared GATE-FC **PASS** and asked to skip a
second Linux/Windows host PC so the software app could be finished. They did
not paste runbook measurements. There is no `docs/phase-logs/gate-fc-result.md`
and no live session URL in this repo. `SEED_HOST_KASM_URL` remains
`https://example.invalid/kasm`. Do not treat this closure as agent-verified
Firecracker/Kasm on metal.

## Pass / fail (read this first)

**PASS** only if all of these are true:

1. The Linux host has working KVM (`/dev/kvm` readable and writable).
2. A Firecracker microVM boots (not QEMU, not Docker-on-the-host, not WSL).
3. Kasm (Workspaces or KasmVNC serving a desktop) is installed **in the guest**.
4. A browser on the host (or another LAN machine) opens a **live desktop** at
   an `https://` or `http://` URL that is **not** `https://example.invalid/kasm`.
5. You can type in that desktop. It must not be a screenshot, a recorded demo,
   or a Kasm instance running on the host OS.

**FAIL** (do not close the gate): KVM missing; Firecracker demo rootfs only
(no Kasm); Kasm installed on the host; URL still `example.invalid`; TCG/QEMU
used instead of Firecracker.

Out of scope for this gate: Matchmaker, Host Agent, Tailscale, Storj, billing.
Guest egress still uses the host's IP — that is a known launch blocker
(architecture §9 item 5), not a reason to fail GATE-FC.

## Machine

- Linux x86_64 or aarch64 (Ubuntu 22.04/24.04 or Debian 12 are fine).
- CPU with virtualization enabled in firmware (Intel VT-x / AMD SVM).
- Recommended for the Kasm guest: **4 vCPU, 8 GiB RAM, 64 GiB disk**.
  Official Kasm single-server minimum is 2 cores / 4 GiB / ~50–75 GiB SSD;
  default workspace images want ~2 cores / 2768 MiB extra.
- Nested Docker *inside the guest* is required for Kasm Workspaces. Nested
  *KVM* is not.

Official references (use these if a command below bit-rots):

- Firecracker getting started: https://github.com/firecracker-microvm/firecracker/blob/main/docs/getting-started.md
- Kasm single-server install: https://kasm.com/docs/latest/install/single_server_install.html

Work in a dedicated directory, e.g. `~/dcl-gate-fc`. Commands assume bash.

---

## 1. Prove KVM

```bash
uname -s -m
# expect: Linux x86_64   OR   Linux aarch64

lsmod | grep kvm
# expect a line with kvm_intel or kvm_amd, plus kvm

ls -l /dev/kvm
[ -r /dev/kvm ] && [ -w /dev/kvm ] && echo "OK" || echo "FAIL"
# expect: OK
```

If `FAIL`, either `sudo usermod -aG kvm "$USER"` and log out, or
`sudo setfacl -m u:${USER}:rw /dev/kvm`, then re-check.

**Record:** paste `uname`, `lsmod | grep kvm`, and the OK/FAIL line into the
result file.

---

## 2. Download the Firecracker binary

```bash
ARCH="$(uname -m)"
release_url="https://github.com/firecracker-microvm/firecracker/releases"
latest=$(basename $(curl -fsSLI -o /dev/null -w %{url_effective} ${release_url}/latest))
echo "Firecracker $latest on $ARCH"
curl -L ${release_url}/download/${latest}/firecracker-${latest}-${ARCH}.tgz | tar -xz
mv release-${latest}-${ARCH}/firecracker-${latest}-${ARCH} firecracker
chmod +x firecracker
./firecracker --version
```

**Expect:** a version line (e.g. `Firecracker v1.x.x`).

---

## 3. Smoke-boot only (does **not** pass the gate)

The official CI rootfs is ~1 GiB and has no desktop/Kasm. Use it only to prove
Firecracker + TAP networking. Follow “Getting a rootfs and Guest Kernel Image”
and “Starting Firecracker” in the getting-started doc (kernel + squashfs →
ext4, TAP `172.16.0.1/30`, guest MAC `06:00:AC:10:00:02`, SSH
`root@172.16.0.2`).

**Expect:** `ssh -i ./ubuntu-*.id_rsa root@172.16.0.2` drops you in a shell.
`reboot` in the guest shuts Firecracker down.

If SSH fails, stop here and put the error in the result file. Do not install
Kasm on the host as a workaround.

---

## 4. Build a Kasm-sized guest disk

The 1 GiB demo disk cannot hold Docker + Kasm. Create a larger ext4, install
Ubuntu/Debian into it, and boot that disk with the **same Firecracker kernel**
from step 3.

Sketch (adjust if you already have a known-good Firecracker rootfs):

```bash
# 64 GiB sparse image
truncate -s 64G kasm-guest.ext4
sudo mkfs.ext4 -F kasm-guest.ext4

# Mount and install a minimal Ubuntu (example: debootstrap)
sudo mkdir -p /mnt/kasm-guest
sudo mount -o loop kasm-guest.ext4 /mnt/kasm-guest
sudo debootstrap --arch=$(dpkg --print-architecture) jammy /mnt/kasm-guest http://archive.ubuntu.com/ubuntu
# On aarch64 use the Ubuntu ports mirror instead.

# Inside the chroot: root password or SSH key, systemd, networking, docker deps.
# You need: python3, curl, ca-certificates, and a kernel with overlayfs/cgroup
# support — the Firecracker CI vmlinux from step 3 is the kernel you will boot.
sudo umount /mnt/kasm-guest
```

If debootstrap is too painful, import a cloud-style Ubuntu rootfs, grow it to
64 GiB (`truncate` + `resize2fs`), and inject an SSH authorized_key the same
way the Firecracker getting-started script does.

Before `InstanceStart`, set machine size (Firecracker defaults are far too
small for Kasm):

```bash
API_SOCKET="/tmp/firecracker.socket"
sudo curl -X PUT --unix-socket "${API_SOCKET}" \
  --data '{"vcpu_count":4,"mem_size_mib":8192,"smt":false}' \
  "http://localhost/machine-config"
```

Point the root drive at `kasm-guest.ext4` instead of the 1 GiB demo. Keep TAP
+ iptables MASQUERADE from the getting-started doc so the guest can pull Docker
images.

**Expect:** SSH into the large guest. `free -h` shows ~8 GiB. `df -h /` shows
tens of GiB free.

---

## 5. Install Kasm **inside the guest**

SSH into the guest. Confirm you are not on the host:

```bash
# in the guest
hostnamectl
cat /proc/1/cgroup
# Firecracker guests typically show a normal systemd tree, not a host docker cgroup.
```

Then follow the current Kasm single-server page (do not invent a tarball name
if the docs moved). As of writing, Ubuntu 22.04/24.04 or Debian 12, then:

```bash
cd /tmp
# Replace the tarball with the filename listed on the Kasm install page.
curl -O https://kasm-static-content.s3.amazonaws.com/kasm_release_1.17.0.7f020d.tar.gz
tar -xf kasm_release_*.tar.gz
sudo bash kasm_release/install.sh
```

**Expect:** installer prints admin and user passwords and says the web app is
on port 443. Save those passwords offline; do not commit them.

Optional lighter path if Workspaces will not fit: install **KasmVNC** in the
guest with a desktop session. That still counts **if** a browser shows a live
Linux desktop from inside Firecracker. Say which path you used in the result
file.

**Must not:** `apt install` / run Kasm on the Linux **host**.

---

## 6. Open the desktop from a browser

On the Linux host (or another machine that can route to the TAP):

```bash
# guest address in the getting-started TAP layout
curl -kI https://172.16.0.2/
```

**Expect:** HTTP 200/302 from Kasm (certificate will be self-signed; `-k` is
fine for the gate).

Open `https://172.16.0.2/` in a browser, accept the warning, log in, launch a
workspace (or the KasmVNC desktop). **Expect:** an interactive desktop — mouse
and keyboard work.

Copy the URL you actually used (include port if not 443). This is the value
that later becomes `SEED_HOST_KASM_URL`. It must not be `example.invalid`.

---

## 7. Report results (required to close the gate later)

Copy `docs/phase-logs/gate-fc-result.example.md` is not in the repo on purpose.
Create **`docs/phase-logs/gate-fc-result.md`** with this exact shape. The
operator closed GATE-FC **without** attaching that file; the filled template
below is the in-runbook record only.

```markdown
# GATE-FC result

- Date (ISO): 2026-09-05
- Operator (human name): operator declared PASS; measurements not attached
- Host OS / kernel / arch: operator declared PASS; measurements not attached
- Firecracker version (`./firecracker --version`): operator declared PASS; measurements not attached
- Guest vCPU / RAM / disk: operator declared PASS; measurements not attached
- Kasm path used: Workspaces / KasmVNC (pick one): operator declared PASS; measurements not attached
- Kasm version or git tag: operator declared PASS; measurements not attached
- Live URL (the one a browser opened): operator declared PASS; measurements not attached
- Screenshot or 10s note of what you did in the desktop: operator declared PASS; measurements not attached
- KVM check output (step 1): operator declared PASS; measurements not attached
- `curl -kI` status line (step 6): operator declared PASS; measurements not attached
- Failures / workarounds: operator declared PASS; measurements not attached
- Human confirms GATE-FC may be marked closed: yes (operator declaration 2026-09-05)
```

Do not commit passwords. Do not invent lab output to replace the lines above.
`SEED_HOST_KASM_URL` stays `https://example.invalid/kasm`. There is still no
live Firecracker/Kasm session in this repo.
