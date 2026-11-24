"""Runs the Pro GPU readiness check against a fake sysfs tree (no real GPU)."""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
SH = SCRIPTS / "pro-gpu-check.sh"
PS1 = SCRIPTS / "pro-gpu-check.ps1"


def _gpu(root: Path, bdf: str, vendor: str, device: str, group: str,
         boot_vga: str, driver: str, card: str, screen: bool) -> None:
    dev = root / "sys/bus/pci/devices" / bdf
    dev.mkdir(parents=True)
    (dev / "class").write_text("0x030000\n")
    (dev / "vendor").write_text(vendor + "\n")
    (dev / "device").write_text(device + "\n")
    (dev / "boot_vga").write_text(boot_vga + "\n")
    drv = root / "sys/bus/pci/drivers" / driver
    drv.mkdir(parents=True, exist_ok=True)
    (dev / "driver").symlink_to(drv)
    grp = root / "sys/kernel/iommu_groups" / group / "devices"
    grp.mkdir(parents=True, exist_ok=True)
    (grp / bdf).symlink_to(dev)
    (dev / "iommu_group").symlink_to(grp.parent)
    drm = root / "sys/class/drm"
    (drm / card).mkdir(parents=True, exist_ok=True)
    (drm / card / "device").symlink_to(dev)
    conn = drm / f"{card}-HDMI-A-1"
    conn.mkdir()
    (conn / "status").write_text("connected\n" if screen else "disconnected\n")


def _tree(root: Path, with_igpu: bool, shared_group: bool = False) -> None:
    (root / "proc").mkdir()
    (root / "proc/cpuinfo").write_text("model name\t: Test CPU\nflags\t\t: fpu vmx\n")
    (root / "proc/cmdline").write_text("root=/dev/x intel_iommu=on iommu=pt\n")
    (root / "sys/bus/pci/drivers/vfio-pci").mkdir(parents=True)
    _gpu(root, "0000:01:00.0", "0x10de", "0x2786", "12", "0", "nvidia", "card1", False)
    if with_igpu:
        _gpu(root, "0000:00:02.0", "0x8086", "0x4680", "0", "1", "i915", "card0", True)
    if shared_group:
        other = root / "sys/bus/pci/devices/0000:02:00.0"
        other.mkdir()
        (other / "class").write_text("0x010802\n")  # NVMe drive
        (root / "sys/kernel/iommu_groups/12/devices/0000:02:00.0").symlink_to(other)


class ProGpuCheckTests(unittest.TestCase):
    def _run(self, **tree) -> str:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "root"
            root.mkdir()
            _tree(root, **tree)
            env = dict(os.environ,
                       PRO_CHECK_SYSROOT=str(root),
                       PRO_CHECK_REPORT=str(Path(tmp) / "report.txt"),
                       PATH="/usr/bin:/bin")  # no nvidia-smi, lspci or qemu
            out = subprocess.run(["bash", str(SH)], env=env, capture_output=True,
                                 text=True, timeout=30).stdout
            self.assertEqual(out, (Path(tmp) / "report.txt").read_text())
            return out

    def test_scripts_parse(self) -> None:
        subprocess.run(["bash", "-n", str(SH)], check=True)
        self.assertTrue(PS1.is_file())

    def test_igpu_plus_card_is_ready(self) -> None:
        out = self._run(with_igpu=True)
        self.assertIn("IOMMU is active: 2 groups", out)
        self.assertIn("vfio-pci driver is loaded", out)
        self.assertIn("2 display devices", out)
        self.assertIn("0000:01:00.0", out)
        self.assertIn("good candidate for Pro", out)
        self.assertIn("screens connected: HDMI-A-1", out)  # the iGPU drives the screen
        self.assertIn("nvidia-smi not found", out)

    def test_single_gpu_fails_display_path(self) -> None:
        out = self._run(with_igpu=False)
        self.assertIn("Only one display device", out)

    def test_shared_iommu_group_is_flagged(self) -> None:
        out = self._run(with_igpu=True, shared_group=True)
        self.assertIn("shares IOMMU group 12 with other devices: 0000:02:00.0", out)

    def test_windows_check_stays_read_only(self) -> None:
        text = PS1.read_text()
        for verb in ("Dismount-VMHostAssignableDevice -", "Add-VMAssignableDevice -",
                     "Set-VMHostPartitionableGpu -", "Enable-WindowsOptionalFeature",
                     "New-VM", "Start-VM", "Disable-PnpDevice"):
            self.assertNotIn(verb, text, verb)


if __name__ == "__main__":
    unittest.main()
