# Baseline test log — Windows host, LAN test kit, quick mode

- **Date:** 2026-11-14
- **Reported by:** project owner (screenshots + pasted terminal output)
- **Purpose:** first run on real hardware; baseline numbers to compare later
  changes against (virtio disk, more vCPU/RAM, signed image, Host Agent path).

## Setup

| Item | Value |
| --- | --- |
| Host | Physical Windows 10 x86-64 PC (French UI), on university Wi-Fi |
| Host LAN address | `192.168.120.240` (Wi-Fi, gateway `192.168.120.1`) |
| Kit | `dcl-lan-test-windows.zip` built from commit `b702a2e`, quick mode |
| Hypervisor | Pinned Weilnetz QEMU `qemu-w64-setup-20260811`, `-accel whpx -M pc` |
| Guest disk | Ubuntu 24.04 cloud image `release-20261017` (SHA-256 `04e8f246…`) + qcow2 overlay, attached as `-hda` (IDE, `/dev/sda`) |
| Guest size | 2 vCPU, 4 GiB RAM (3.8 GiB visible), 20 GB disk (19 GB root) |
| Guest network | QEMU SLIRP user-net, `hostfwd tcp:0.0.0.0:8443 -> guest 8443` |
| Desktop | XFCE + KasmVNC 1.3.4 via `provision-kasm.sh`, basic auth from SMBIOS OEM string |
| Client | Mac on the same network, Brave browser |

## Functional result: PASS

| Step | Result |
| --- | --- |
| Admin prompt, WHPX check, QEMU install, Ubuntu download + SHA-256 | OK |
| Firewall rule (TCP 8443, local subnet only) | OK |
| First boot: cloud-init installs desktop | OK, desktop answered after **394 s** |
| Desktop asks for a password (no auth → HTTP 401) | OK |
| Second device opens `https://192.168.120.240:8443`, logs in as `ubuntu` | OK, XFCE desktop shown, terminal usable |

Notes from the session:

- One attempt from the Mac got `ERR_ADDRESS_UNREACHABLE` (no ARP reply from
  the PC). A later attempt on the same network worked, so the university
  Wi-Fi does not isolate clients; most likely the PC was asleep or the test
  was not running at that moment.
- Pasting multi-line commands through KasmVNC into `xfce4-terminal` inserted
  bracketed-paste markers (`^[[200~` … `~`). Workaround: type
  `bind 'set enable-bracketed-paste off'` first.
- Not yet checked on this PC: a **second run** of the kit (open issue 3b in
  `docs/issues.md`) and the report file contents.

## Performance baseline (inside the guest)

Commands (run in the guest terminal):

```bash
sudo apt-get update -qq && sudo apt-get install -y -qq sysbench fio
nproc; free -h | head -2; df -h / | tail -1
sysbench cpu --threads=$(nproc) --time=20 run | grep "events per second"
sysbench memory --memory-total-size=4G run | grep "transferred"
fio --name=seq --filename=/tmp/t --size=1G --rw=readwrite --bs=1M --direct=1 --runtime=20 --time_based --group_reporting | grep -E "READ:|WRITE:"
fio --name=rnd --filename=/tmp/t --size=1G --rw=randread --bs=4k --direct=1 --iodepth=32 --ioengine=libaio --runtime=20 --time_based | grep IOPS; rm -f /tmp/t
```

| Metric | Baseline | Reading |
| --- | --- | --- |
| CPU (sysbench, 2 threads) | **1,723.82 events/s** (~860 per core) | In line with a typical cloud vCPU; WHPX overhead looks small |
| Memory (sysbench, 1 KiB blocks) | **4,191 MiB/s** | Fine for desktop use |
| Disk sequential, 1 MiB, mixed R/W | **127 MiB/s read + 133 MiB/s write** | Good for large files |
| Disk random 4 KiB read, QD32 | **1,426 IOPS** (5.7 MiB/s) | **Weak spot.** Likely the IDE `-hda` attachment; Linux kit uses virtio (`/dev/vda`) |
| Internet download (guest) | **~433 kB/s average (~3.5 Mbit/s)**, 229 kB/s (~1.8 Mbit/s) at the end; stopped at 60.7 of 100 MB after 2 min 23 s | **Slow.** Cause not isolated yet: university Wi-Fi, the OVH server, or QEMU SLIRP user-net. Needs the same download on the Windows host to compare. (First `speed.cloudflare.com` attempt returned nothing.) |
| `glxgears` (software OpenGL, 2 vCPU) | **518–672 FPS**, mostly 530–610 | Drawing inside the VM is fast (CPU only, no GPU). This is not the frame rate the browser receives; KasmVNC streams far fewer frames. |
| Interactive feel (typing, dragging windows) | not reported yet | Owner to note: instant / slight delay / laggy |

Raw output (trimmed of `apt` lines):

```
== VM size
2
               total        used        free      shared  buff/cache   available
Mem:           3.8Gi       599Mi       2.7Gi       7.6Mi       807Mi       3.2Gi
/dev/sda1        19G  2.6G   16G  14% /
== CPU
    events per second:  1723.82
== Memory
4096.00 MiB transferred (4191.11 MiB/sec)
== Disk seq
   READ: bw=127MiB/s (133MB/s), 127MiB/s-127MiB/s (133MB/s-133MB/s), io=2541MiB (2664MB), run=20004-20004msec
  WRITE: bw=133MiB/s (139MB/s), 133MiB/s-133MiB/s (139MB/s-139MB/s), io=2656MiB (2785MB), run=20004-20004msec
== Disk 4k
  read: IOPS=1426, BW=5707KiB/s (5844kB/s)(111MiB/20002msec)
== Internet
0.0 Mbit/s
```

Internet retry and glxgears (same session):

```
$ curl -L -o /dev/null -w "http %{http_code}, %{size_download} bytes, %{speed_download} B/s\n" https://proof.ovh.net/files/100Mb.dat
 60  100M   60 60.7M    0     0   433k      0  0:03:56  0:02:23  0:01:33  229k^C
$ glxgears
2671 frames in 5.0 seconds = 534.172 FPS
2809 frames in 5.0 seconds = 561.644 FPS
2663 frames in 5.0 seconds = 532.537 FPS
2895 frames in 5.0 seconds = 578.720 FPS
2705 frames in 5.0 seconds = 540.971 FPS
2717 frames in 5.0 seconds = 543.322 FPS
2621 frames in 5.0 seconds = 524.198 FPS
2740 frames in 5.0 seconds = 547.895 FPS
2758 frames in 5.0 seconds = 551.318 FPS
2591 frames in 5.0 seconds = 517.965 FPS
2826 frames in 5.0 seconds = 564.886 FPS
2824 frames in 5.0 seconds = 564.760 FPS
2793 frames in 5.0 seconds = 558.573 FPS
2768 frames in 5.0 seconds = 553.538 FPS
2850 frames in 5.0 seconds = 569.934 FPS
3217 frames in 5.0 seconds = 643.369 FPS
3359 frames in 5.0 seconds = 671.721 FPS
3269 frames in 5.0 seconds = 653.728 FPS
2833 frames in 5.0 seconds = 566.273 FPS
3224 frames in 5.0 seconds = 644.594 FPS
3039 frames in 5.0 seconds = 607.643 FPS
2999 frames in 5.0 seconds = 599.310 FPS
3070 frames in 5.0 seconds = 613.643 FPS
```

## Compare against this baseline

When re-running, keep the same commands and record a new file in
`docs/test-logs/` with the date and what changed. Planned comparisons:

1. Windows disk as virtio instead of `-hda` (`lan-test.ps1` and
   `host-agent/src-tauri/src/qemu_whpx.rs`): expect random 4 KiB IOPS to rise.
2. `LAN_TEST_VCPUS=4`, `LAN_TEST_MEMORY_GIB=8`: CPU should roughly double.
3. Host baseline on the same PC (test stopped): `winsat cpu -encryption`,
   `winsat disk -drive c`.
   Same internet download on the host, to tell Wi-Fi from SLIRP:
   `curl.exe -L -o NUL -w "%{speed_download} B/s" https://proof.ovh.net/files/100Mb.dat`
4. Linux host (KVM) with the Linux kit.
5. Signed Packer image instead of quick mode.
