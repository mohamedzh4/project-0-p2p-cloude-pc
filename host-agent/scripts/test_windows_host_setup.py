"""Checks the volunteer Windows kit without running PowerShell."""

from __future__ import annotations

import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
PS1 = SCRIPTS / "windows-host-setup.ps1"
CMD = SCRIPTS / "setup.cmd"
PACK = SCRIPTS / "pack-friend-kit.sh"
EXAMPLE = SCRIPTS / "friend.env.example"


class WindowsHostSetupTests(unittest.TestCase):
    def test_kit_files_exist(self) -> None:
        self.assertTrue(PS1.is_file())
        self.assertTrue(CMD.is_file())
        self.assertTrue(PACK.is_file())
        self.assertTrue(EXAMPLE.is_file())

    def test_script_does_work_software_can_do(self) -> None:
        text = PS1.read_text(encoding="utf-8")
        self.assertIn("HypervisorPlatform", text)
        self.assertIn("7842", text)
        self.assertIn("Tailscale.Tailscale", text)
        self.assertIn("qemu.weilnetz.de/w64/qemu-w64-setup-20260811.exe", text)
        self.assertIn(
            "5bcf9eed634e8575a37b74f445af41a2fe4106da512d0c30c368301d4c105037fdfab40a5287367a28a957624cddebbc8c07e16c88ab6634f554cdf3d16bf543",
            text,
        )
        self.assertIn("qemu-img convert", text)
        self.assertIn("Get-FileHash", text)
        self.assertIn("HOST_AGENT_URL from friend.env", text)
        self.assertIn("-R", text)
        self.assertIn("17842", text)
        self.assertIn("standby-timeout-ac", text)
        self.assertIn("RunAs", text)
        self.assertIn("do not log in to tailscale.com", text.lower())

    def test_script_does_not_claim_bios_or_cgnat(self) -> None:
        text = PS1.read_text(encoding="utf-8")
        self.assertIn("VirtualizationFirmwareEnabled", text)
        # A running hypervisor makes that flag read False; skip the check then.
        check = text.split("function Assert-FirmwareVirtualization", 1)[1].split("\nfunction ", 1)[0]
        self.assertLess(check.index("HypervisorPresent"), check.index("VirtualizationFirmwareEnabled -eq"))
        self.assertIn("CGNAT", text)
        self.assertIn("cannot", text.lower())

    def test_example_has_no_bootstrap_token(self) -> None:
        text = EXAMPLE.read_text(encoding="utf-8")
        self.assertIn("HOST_BOOTSTRAP_TOKEN=", text)
        token_line = [
            line
            for line in text.splitlines()
            if line.startswith("HOST_BOOTSTRAP_TOKEN=")
        ][0]
        self.assertEqual(token_line, "HOST_BOOTSTRAP_TOKEN=")
        self.assertNotIn("storj", text.lower())

    def test_cmd_bypasses_execution_policy(self) -> None:
        cmd = CMD.read_text(encoding="utf-8")
        self.assertIn("ExecutionPolicy Bypass", cmd)
        self.assertIn("windows-host-setup.ps1", cmd)


if __name__ == "__main__":
    unittest.main()
