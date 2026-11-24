"""Checks the Linux kit without running it."""

from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

SH = Path(__file__).resolve().parent / "linux-host-setup.sh"


class LinuxHostSetupTests(unittest.TestCase):
    def test_script_parses(self) -> None:
        subprocess.run(["bash", "-n", str(SH)], check=True)

    def test_an_older_host_agent_is_updated_not_kept(self) -> None:
        text = SH.read_text(encoding="utf-8")
        # The old agent is stopped before the new one is installed.
        self.assertLess(text.index("\nstop_running_agent\n"), text.index("apt-get install -y --reinstall"))
        self.assertIn("--allow-downgrades", text)
        self.assertNotIn('apt-get install -y "$deb"', text)

    def test_the_launch_does_not_overwrite_the_agents_own_log(self) -> None:
        text = SH.read_text(encoding="utf-8")
        self.assertNotIn('>"$APP_DIR/host-agent.log"', text)


if __name__ == "__main__":
    unittest.main()
