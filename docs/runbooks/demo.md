# Cloud-computer demo runbook

This is the shortest production-shaped path for the Standard demo:

`browser -> Caddy -> web/Matchmaker -> Host Agent -> QEMU KVM or WHPX ->
Ubuntu guest -> Headscale -> KasmVNC/XFCE`

The browser does not need Tailscale. Its short-lived session link is exchanged
for an HttpOnly cookie on the dedicated desktop origin.

## 1. Required machines

- Linux VPS with a public IP, this repository checked out on it
  (examples below use `lab.example.com` and `/home/dcl/dcl`; use your own)
- Ubuntu 22.04/24.04 x86-64 build/host PC with sudo and read/write `/dev/kvm`
- Windows 10/11 x86-64 host with WHPX enabled for the second host proof

These DNS records (your domain in place of `lab.example.com`) must all
resolve to the VPS:

- `lab.example.com`
- `desktop.lab.example.com`
- `storage.lab.example.com`
- `mesh.lab.example.com`

## 2. Deploy the VPS

Copy/pull this repository onto the VPS, then run, with your domain:

```bash
cd /home/dcl/dcl
DCL_DOMAIN=lab.example.com bash scripts/vps-deploy.sh
```

The service account defaults to the user running the script (`DCL_USER`).

The script preserves existing generated secrets, upgrades the database, binds
PostgreSQL/Headscale/renterd/Matchmaker to loopback, and publishes only Caddy
on HTTPS.

Expected public checks:

```bash
curl -fsS https://lab.example.com/
curl -fsS https://lab.example.com/api/docs >/dev/null
curl -fsS https://mesh.lab.example.com/health
```

Never commit the generated `.env`, renterd seed, Headscale keys, or signing
private key.

## 3. Build and sign the Standard guest

Run on the Linux KVM machine:

```bash
sudo apt-get update
sudo apt-get install -y qemu-system-x86 qemu-utils xorriso
# Install Packer >= 1.10 from HashiCorp's package repository.

openssl genpkey -algorithm ED25519 -out "$HOME/dcl-image-signing.pem"
chmod 600 "$HOME/dcl-image-signing.pem"

cd guest-images
export IMAGE_SIGNING_PRIVATE_KEY="$HOME/dcl-image-signing.pem"
./scripts/build.sh standard
```

The resulting `output/standard/rootfs.qcow2` contains Ubuntu, XFCE, Tailscale,
rclone, and a systemd-managed KasmVNC listener on HTTPS 8443. The checked-in
package and Ubuntu checksums are verified during the build.

Copy `output/standard/env.snippet` values into the VPS Matchmaker environment,
then restart Matchmaker. Distribute `rootfs.qcow2`, its raw-image digest and
signature, and the signing public key through a private host kit or an HTTPS
artifact URL.

## 4. Build Host Agent packages

Run the `Host Agent packages` GitHub Actions workflow. Download:

- `host-agent-linux` for the Ubuntu host
- `host-agent-windows` for the Windows host

Do not commit generated installers.

## 5. Configure the Linux host

Prepare `host-agent/scripts/friend.env` from
`linux.friend.env.example`, using:

```dotenv
MATCHMAKER_URL=https://lab.example.com/api
HEADSCALE_LOGIN_SERVER=https://mesh.lab.example.com
```

Fill the host bootstrap token and real image signing values. Put the Linux
Host Agent `.deb` and `rootfs.qcow2` beside the setup script, then run:

```bash
bash host-agent/scripts/linux-host-setup.sh
```

Open Host Agent, choose shared CPU/RAM, enable sharing, and register. Approve
the pending host using the Matchmaker operator token. The VPS must be able to
reach the agent URL; the supplied setup supports a loopback reverse SSH tunnel.

## 6. Configure the Windows host

Prepare the Windows friend kit with `pack-friend-kit.sh`, including the NSIS
Host Agent installer and `rootfs.qcow2`. On Windows, right-click PowerShell as
Administrator and run `setup.cmd`.

The setup enables Windows Hypervisor Platform, installs QEMU and Tailscale,
verifies the signed image, and configures the agent. Reboot if Windows asks.
Register and approve this host separately.

## 7. Browser proof

1. Open `https://lab.example.com`.
2. Sign up or sign in.
3. Choose an office/code Standard workload and request a computer.
4. Wait for the cold VM to boot (up to five minutes).
5. Click **Open desktop**.
6. Confirm an XFCE desktop appears at the desktop subdomain without a second
   login or a Tailscale client.
7. Create a file under `/home/ubuntu/Persistent`, end the session, and confirm
   the Host Agent tears down QEMU.
8. Repeat on the other host after pausing the first host.

## Proof boundary

Unit/integration tests can prove selection, signing, launch arguments,
bootstrap, token resolution, proxying, and teardown. Only the two real-host
tests above can prove KVM/WHPX availability and browser pixels end to end.
Do not label either host confirmed until that test has been observed.
