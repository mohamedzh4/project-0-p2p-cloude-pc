"""Checks the LAN hardware test kit without booting a VM or running PowerShell."""

from __future__ import annotations

import os
import re
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
KIT = SCRIPTS / "lan-test"
SH = KIT / "lan-test.sh"
PS1 = KIT / "lan-test.ps1"
CMD = KIT / "lan-test.cmd"
PACK = KIT / "pack-lan-test-kit.sh"
README = KIT / "README-FRIEND.txt"
WINDOWS_SETUP = SCRIPTS / "windows-host-setup.ps1"


def _pinned(text: str, name: str) -> str:
    match = re.search(rf'\$script:{name}\s*=\s*"([^"]+)"', text)
    assert match is not None, name
    return match.group(1)


class LanTestKitTests(unittest.TestCase):
    def test_kit_files_exist(self) -> None:
        for path in (SH, PS1, CMD, PACK, README, KIT / "lan-test.env.example"):
            self.assertTrue(path.is_file(), path)

    def test_shell_scripts_parse(self) -> None:
        for path in (SH, PACK):
            subprocess.run(["bash", "-n", str(path)], check=True)

    def test_cmd_launcher_uses_crlf_and_bypass(self) -> None:
        raw = CMD.read_bytes()
        self.assertIn(b"\r\n", raw)
        self.assertNotIn(b"\n", raw.replace(b"\r\n", b""))
        self.assertIn(b"-ExecutionPolicy Bypass", raw)
        self.assertIn(b"lan-test.ps1", raw)

    def test_both_kits_boot_a_throwaway_copy_with_lan_password(self) -> None:
        for path in (SH, PS1):
            text = path.read_text(encoding="utf-8")
            self.assertIn("-snapshot", text, path)  # signed mode
            self.assertIn("hostfwd=tcp:0.0.0.0:", text, path)
            self.assertIn("-:8443", text, path)
            self.assertIn("type=11,path=", text, path)
            self.assertIn("dcl-lan-password=", text, path)
            self.assertNotIn("-fw_cfg", text, path)
            self.assertIn("-name", text, path)
            self.assertIn("dcl-lan-test", text, path)
            self.assertNotIn("tcg", text.lower(), path)

    def test_linux_matches_host_agent_kvm_argv(self) -> None:
        text = SH.read_text(encoding="utf-8")
        for flag in ("-enable-kvm", "-machine q35", "-cpu host", "if=virtio,format=raw",
                     "virtio-net-pci,netdev=net0", "-no-reboot"):
            self.assertIn(flag, text)

    def test_windows_matches_host_agent_whpx_argv(self) -> None:
        text = PS1.read_text(encoding="utf-8")
        for flag in ('"-accel", "whpx"', '"-M", "pc"', '"ich9-usb-ehci1"', '"usb-tablet"',
                     '"-hda"', '"virtio-net-pci,netdev=net0"'):
            self.assertIn(flag, text)

    def test_windows_uses_the_same_pinned_qemu_as_host_setup(self) -> None:
        lan = PS1.read_text(encoding="utf-8")
        setup = WINDOWS_SETUP.read_text(encoding="utf-8")
        for name in ("QemuInstallerUrl", "QemuInstallerSha512", "QemuVersion", "ImageDir"):
            self.assertEqual(_pinned(lan, name), _pinned(setup, name), name)

    def test_image_digest_is_checked_before_boot(self) -> None:
        sh = SH.read_text(encoding="utf-8")
        ps1 = PS1.read_text(encoding="utf-8")
        self.assertLess(sh.index("sha256sum"), sh.index("qemu-system-x86_64 \"${qemu_args[@]}\""))
        self.assertIn("Get-FileHash -LiteralPath $Raw -Algorithm SHA256", ps1)
        self.assertLess(ps1.index("Install-GuestDisk -QemuDir"), ps1.index("Start-Process -FilePath $qemuSys"))

    def test_firewall_is_lan_only_and_removed(self) -> None:
        ps1 = PS1.read_text(encoding="utf-8")
        self.assertIn("-RemoteAddress LocalSubnet", ps1)
        finally_block = ps1[ps1.index("finally {\n    if ($script:Result"):]
        self.assertIn("Remove-NetFirewallRule", finally_block)
        self.assertIn("Stop-Process", finally_block)
        sh = SH.read_text(encoding="utf-8")
        cleanup = sh[sh.index("cleanup() {"):sh.index("trap cleanup EXIT")]
        self.assertIn("ufw delete allow", cleanup)
        self.assertIn('kill "$QEMU_PID"', cleanup)

    def test_password_is_shown_but_never_written_to_the_report(self) -> None:
        sh = SH.read_text(encoding="utf-8")
        for line in sh.splitlines():
            if "$PASSWORD" in line and line.lstrip().startswith("log "):
                self.fail(f"password logged: {line}")
        self.assertIn("Password:  $PASSWORD", sh)
        ps1 = PS1.read_text(encoding="utf-8")
        for line in ps1.splitlines():
            if "$password" in line and "Write-TestLog" in line:
                self.fail(f"password logged: {line}")
        self.assertIn('Write-Host "  Password:  $password"', ps1)

    def test_quick_mode_pins_the_same_dated_ubuntu_image(self) -> None:
        sh = SH.read_text(encoding="utf-8")
        ps1 = PS1.read_text(encoding="utf-8")
        sh_url = re.search(r'^UBUNTU_URL="([^"]+)"', sh, re.M).group(1)
        sh_sha = re.search(r'^UBUNTU_SHA256="([0-9a-f]{64})"', sh, re.M).group(1)
        self.assertEqual(sh_url, _pinned(ps1, "UbuntuUrl"))
        self.assertEqual(sh_sha, _pinned(ps1, "UbuntuSha256"))
        self.assertRegex(sh_url, r"/release-\d{8}/")
        for text in (sh, ps1):
            self.assertIn("DCL-QUICK-SETUP-FAILED", text)
            self.assertIn("setup-done", text)

    def test_both_fail_closed_without_password_prompt(self) -> None:
        self.assertIn("Stopped the VM so the desktop is not open to the network without a password", SH.read_text(encoding="utf-8"))
        self.assertIn("Stopped the VM so the desktop is not open to the network without a password", PS1.read_text(encoding="utf-8"))

    def test_quick_seed_runs_the_real_provision_script(self) -> None:
        tpl = (KIT / "quick-user-data.in").read_text(encoding="utf-8")
        self.assertTrue(tpl.startswith("#cloud-config"))
        for marker in ("@PROVISION_KASM_B64@", "@KASM_VNC_DEB_URL@", "@KASM_VNC_DEB_SHA256@",
                       "DCL-QUICK-SETUP-OK", "DCL-QUICK-SETUP-FAILED", "systemctl start dcl-kasmvnc.service"):
            self.assertIn(marker, tpl)

    def test_pack_quick_mode_builds_seed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            env = dict(os.environ, LAN_TEST_KIT_DIR=tmp)
            subprocess.run(["bash", str(PACK)], check=True, env=env, capture_output=True)
            iso = (Path(tmp) / "seed.iso").read_bytes()
            self.assertIn(b"CIDATA", iso.upper()[32768:34816])
            with zipfile.ZipFile(Path(tmp) / "dcl-lan-test-windows.zip") as z:
                names = set(z.namelist())
                self.assertIn("dcl-lan-test-windows/seed.iso", names)
                env_text = z.read("dcl-lan-test-windows/lan-test.env").decode()
                self.assertIn("LAN_TEST_MODE=quick", env_text)
                self.assertNotIn("IMAGE_STANDARD_DIGEST", env_text)

    def test_pack_builds_both_zips(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            env = dict(os.environ, LAN_TEST_KIT_DIR=tmp)
            subprocess.run(
                ["bash", str(PACK), "--digest", "a" * 64,
                 "--qcow2-url", "https://example.com/rootfs.qcow2"],
                check=True, env=env, capture_output=True,
            )
            with zipfile.ZipFile(Path(tmp) / "dcl-lan-test-linux.zip") as z:
                names = set(z.namelist())
                self.assertIn("dcl-lan-test-linux/lan-test.sh", names)
                env_text = z.read("dcl-lan-test-linux/lan-test.env").decode()
                self.assertIn("IMAGE_STANDARD_DIGEST=" + "a" * 64, env_text)
                self.assertIn("IMAGE_QCOW2_URL=https://example.com/rootfs.qcow2", env_text)
            with zipfile.ZipFile(Path(tmp) / "dcl-lan-test-windows.zip") as z:
                names = set(z.namelist())
                self.assertIn("dcl-lan-test-windows/lan-test.ps1", names)
                self.assertIn("dcl-lan-test-windows/lan-test.cmd", names)
                self.assertNotIn("dcl-lan-test-windows/lan-test.sh", names)

    def test_pack_refuses_without_digest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            env = dict(os.environ, LAN_TEST_KIT_DIR=tmp)
            result = subprocess.run(
                ["bash", str(PACK), "--digest", "nothex"],
                env=env, capture_output=True, text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("digest", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
