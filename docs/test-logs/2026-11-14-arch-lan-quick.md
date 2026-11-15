# Test log — Arch Linux host, LAN test kit, quick mode

- **Date:** 2026-11-14
- **Reported by:** project owner (photo of the friend's terminal)
- **Kit:** `dcl-lan-test-linux.zip` with Arch (pacman) support, quick mode,
  before the LAN-address fix below.

## Setup

| Item | Value |
| --- | --- |
| Host | Physical HP laptop, Arch Linux, x86-64 |
| Network | Both PCs on a shared hotspot; host `wlan0` = `10.49.110.198/24` |
| Other interfaces on the host | `wg0` WireGuard `10.13.13.1/24`; Docker `docker0`, `br-…` |
| Firewall | ufw (iptables-nft), incoming policy drop |
| Hypervisor | Distro QEMU (`qemu-base`) with KVM |

## Result: PASS after a manual firewall rule

| Step | Result |
| --- | --- |
| App ran, VM booted, desktop set up | OK |
| On the host: `curl -k https://10.49.110.198:8443` | **401** (desktop asks for a password) |
| QEMU listening | `0.0.0.0:8443` |
| Other PC opens the link | **Failed** until `sudo ufw allow 8443/tcp` on the host, then **worked** |

## Cause and fix

The app opened ufw only for the network of the address it chose, and it
chose that address from the default route. With WireGuard up, the default
route most likely went through `wg0`, so the rule allowed `10.13.13.0/24`
instead of the hotspot's `10.49.110.0/24`. (The printed link was not
captured; the next report will show it.)

Fix in `lan-test.sh` (after this run): only Wi-Fi/Ethernet interfaces are
used, VPN/Docker/bridge interfaces are skipped, every LAN address is
printed, and ufw/firewalld are opened for each LAN network and closed on
exit. Unit test `test_lan_address_skips_vpn_and_docker` replays these routes.

## Performance (inside the guest)

Same commands as the Windows baseline
(`2026-11-14-windows-lan-quick-baseline.md`). Guest: 2 vCPU, 3.8 GiB RAM,
root on **`/dev/vda1` (virtio)**.

| Metric | Arch / KVM (this run) | Windows / WHPX (baseline) | Difference |
| --- | --- | --- | --- |
| CPU, sysbench 2 threads | **1,937 events/s** | 1,724 | +12 % |
| Memory, sysbench | **4,668 MiB/s** | 4,191 | +11 % |
| Disk sequential 1 MiB, mixed R/W | **1,206 MiB/s read + 1,200 MiB/s write** | 127 + 133 | ~9× |
| Disk random 4 KiB read, QD32 | **88,700 IOPS** (346 MiB/s) | 1,426 | **~62×** |
| Internet download (60 s) | **~8.2 Mbit/s** (1,019,875 B/s, 61 MB) | ~3.5 Mbit/s | different networks |
| `glxgears` | **827–1,039 FPS** | 518–672 | +~60 % |

Reading:

- The two hosts are different PCs, so CPU, memory and FPS gaps are mostly
  hardware. The disk gap is far too large for that: Linux attaches the
  guest disk as **virtio** (`/dev/vda`), Windows as emulated **IDE**
  (`-hda`, `/dev/sda`). Switching Windows to virtio (`lan-test.ps1` and
  `host-agent/src-tauri/src/qemu_whpx.rs`) is the obvious next comparison.
- Disk numbers may be helped by the host's page cache: QEMU's default
  cache mode lets the 1 GiB test file sit in host RAM, even with
  `--direct=1` inside the guest. Fine for comparing setups, not a raw
  SSD figure.
- Internet: the Windows run was on university Wi-Fi, this one on a phone
  hotspot, so the numbers can't separate network from QEMU user-net yet.
- Interactive feel (typing, dragging): not reported yet.

Raw output:

```
== VM size
2
               total        used        free      shared  buff/cache   available
Mem:           3.8Gi       605Mi       2.7Gi        12Mi       778Mi       3.2Gi
/dev/vda1        19G  2.6G   16G  14% /
== CPU
    events per second:  1937.11
== Memory
4096.00 MiB transferred (4667.64 MiB/sec)
== Disk seq
   READ: bw=1206MiB/s (1264MB/s), 1206MiB/s-1206MiB/s (1264MB/s-1264MB/s), io=23.6GiB (25.3GB), run=20001-20001msec
  WRITE: bw=1200MiB/s (1259MB/s), 1200MiB/s-1200MiB/s (1259MB/s-1259MB/s), io=23.4GiB (25.2GB), run=20001-20001msec
== Disk 4k
  read: IOPS=88.7k, BW=346MiB/s (363MB/s)(6929MiB/20001msec)
== Internet
http 200, 61194186 bytes, 1019875 B/s
== glxgears (20 s)
4812 frames in 5.0 seconds = 962.315 FPS
4137 frames in 5.0 seconds = 827.383 FPS
5196 frames in 5.0 seconds = 1039.169 FPS
```

Note: pasting the install line together with the benchmark lines lost the
benchmark lines (the installer read them from the keyboard); run the
install line on its own first.

## Host vs guest (same Arch PC)

Same benchmark run on the Arch host itself (test stopped), with three
changes so the numbers compare: disk file in `$HOME` (Arch `/tmp` is RAM),
CPU also with 2 threads (the guest has 2 vCPU), `glxgears` forced to
software rendering. Host: 8 threads, 15 GiB RAM (10 GiB in use by other
apps during the run), NVMe SSD.

| Metric | Host | Guest | Guest vs host |
| --- | --- | --- | --- |
| CPU, 2 threads | 1,766 events/s | 1,937 | ~same (guest even higher: noise, host busy) |
| CPU, all 8 threads | 5,512 events/s | — | guest uses 2 of 8 threads |
| Memory | 5,162 MiB/s | 4,668 | ~90 % |
| Disk sequential, mixed R/W | 613 + 611 MiB/s | 1,206 + 1,200 | guest **inflated** by host page cache |
| Disk random 4 KiB read | 198,000 IOPS | 88,700 | ~45 % |
| Internet (same hotspot) | ~8.5 Mbit/s (1,058,125 B/s) | ~8.2 Mbit/s | **~same: the hotspot is the limit** |
| `glxgears`, software | 387–419 FPS | 827–1,039 | not comparable (see below) |

Reading:

- **CPU: no measurable KVM overhead** at 2 threads.
- **Disk sequential:** the guest beating the SSD confirms that QEMU's
  default host cache serves the 1 GiB test file from RAM. Guest disk
  numbers are good for comparing setups, not as SSD speed.
- **Random reads:** the guest gets about half the host's IOPS even with
  that cache help: the cost of the qcow2 overlay + virtio path.
- **Internet:** host and guest match, so on this network QEMU user-net
  (SLIRP) is not the bottleneck (issue 10).
- **glxgears:** the host draws in software *and* goes through the desktop
  compositor (XWayland); the guest draws into KasmVNC's framebuffer with
  no compositor. Different work, so drop this metric for host-vs-guest.

Raw host output:

```
== Host size
8
               total        used        free      shared  buff/cache   available
Mem:            15Gi        10Gi       1.1Gi       1.1Gi       4.8Gi       4.5Gi
/dev/nvme0n1p3  202G  147G   45G  77% /home
== CPU (2 threads, same as VM)
    events per second:  1766.06
== CPU (all threads)
    events per second:  5511.63
== Memory
4096.00 MiB transferred (5162.49 MiB/sec)
== Disk seq
   READ: bw=613MiB/s (643MB/s), 613MiB/s-613MiB/s (643MB/s-643MB/s), io=12.0GiB (12.9GB), run=20001-20001msec
  WRITE: bw=611MiB/s (641MB/s), 611MiB/s-611MiB/s (641MB/s-641MB/s), io=11.9GiB (12.8GB), run=20001-20001msec
== Disk 4k
  read: IOPS=198k, BW=774MiB/s (812MB/s)(15.1GiB/20001msec)
== Internet
http 200, 63487982 bytes, 1058125 B/s
== glxgears software (20 s, same as VM)
1934 frames in 5.0 seconds = 386.717 FPS
2098 frames in 5.0 seconds = 419.464 FPS
2068 frames in 5.0 seconds = 413.410 FPS
```

## Still to collect

- The report file (`lan-test-report-*.txt`) from this PC.
- Interactive feel (typing, dragging windows).
- After testing: `sudo ufw delete allow 8443/tcp` on the friend's PC (the
  manual rule stays until removed).
