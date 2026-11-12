#!/usr/bin/env bash
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq sudo iproute2 qemu-utils curl python3 openssl procps >/dev/null
useradd -m friend
echo "friend ALL=(ALL) NOPASSWD:ALL" >/etc/sudoers.d/friend
install -m 0755 /e2e/fake-qemu /usr/local/bin/qemu-system-x86_64
touch /dev/kvm && chmod 666 /dev/kvm
{ grep -v '^flags' /proc/cpuinfo || true; } | head -n 20 >/tmp/cpuinfo
echo "flags : fpu vme de pse msr vmx sse sse2" >>/tmp/cpuinfo
echo "model name : Fake test CPU" >>/tmp/cpuinfo
chmod 644 /tmp/cpuinfo
sed 's/^MemAvailable:.*/MemAvailable:   16777216 kB/' /proc/meminfo >/tmp/meminfo && mount --bind /tmp/meminfo /proc/meminfo
openssl req -x509 -newkey rsa:2048 -nodes -subj /CN=guest -days 1 -keyout /tmp/key.pem -out /tmp/cert.pem 2>/dev/null
chmod 644 /tmp/key.pem
KIT=/home/friend/kit; mkdir -p $KIT
cp /e2e/lan-test.sh $KIT/
qemu-img create -q -f qcow2 $KIT/rootfs.qcow2 64M
qemu-img convert -O raw $KIT/rootfs.qcow2 /tmp/ref.img
echo "IMAGE_STANDARD_DIGEST=$(sha256sum /tmp/ref.img | cut -d' ' -f1)" >$KIT/lan-test.env
chown -R friend:friend $KIT
echo "setup done: $(uname -m), $(nproc) threads, $(awk '/MemAvailable/{print int($2/1048576)}' /proc/meminfo) GiB free"
sudo -u friend -H env DCL_LAN_TEST_CPUINFO=/tmp/cpuinfo bash /e2e/driver.sh
