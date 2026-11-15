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

    def test_linux_supports_apt_and_pacman(self) -> None:
        text = SH.read_text(encoding="utf-8")
        self.assertIn("want_pkgs=(qemu-system-x86 qemu-utils curl)", text)
        self.assertIn("want_pkgs=(qemu-base curl)", text)
        self.assertIn('pacman -S --needed --noconfirm "${want_pkgs[@]}"', text)
        # Only packages that were missing are recorded for --cleanup.
        self.assertIn('dpkg -s "$p" >/dev/null 2>&1 || echo "$p" >>"$PKG_STATE"', text)
        self.assertIn('pacman -Q "$p" >/dev/null 2>&1 || echo "$p" >>"$PKG_STATE"', text)
        self.assertNotIn("pacman -Sy ", text)  # no partial upgrades on Arch
        self.assertIn("--add-rich-rule", text)
        self.assertIn("--remove-rich-rule", text)
        self.assertNotIn("--permanent", text)

    def _run_lan_selection(self, routes: str, default_dev: str) -> str:
        """Run the app's LAN address block with a fake `ip` command."""
        text = SH.read_text(encoding="utf-8")
        block = text[text.index("lan_networks() {"):text.index('log "LAN address: $LAN_IP"')]
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / "ip"
            fake.write_text(
                "#!/bin/sh\n"
                'case "$*" in\n'
                f"  *'route show'*) cat <<'EOF'\n{routes}\nEOF\n;;\n"
                f"  *'route get'*) echo '1.1.1.1 via 10.13.13.2 dev {default_dev} src 10.13.13.1 uid 1000' ;;\n"
                "esac\n"
            )
            fake.chmod(0o755)
            script = (
                # No -u: macOS ships bash 3.2, where empty arrays trip -u.
                "set -eo pipefail\n"
                "log() { echo \"LOG $*\"; }\nfail() { echo \"FAIL $*\"; exit 1; }\n"
                + block
                + 'echo "MAIN=$LAN_IP"; echo "OTHERS=${LAN_IPS[*]}"; echo "NETS=${LAN_SUBNETS[*]}"\n'
            )
            env = dict(os.environ, PATH=f"{tmp}:{os.environ['PATH']}")
            out = subprocess.run(["bash", "-c", script], env=env, capture_output=True, text=True)
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            return out.stdout

    def test_lan_address_skips_vpn_and_docker(self) -> None:
        # Real routes from the Arch PC on 2026-11-14 (WireGuard + Docker).
        routes = "\n".join([
            "10.49.110.0/24 dev wlan0 proto kernel scope link src 10.49.110.198 metric 600",
            "10.13.13.0/24 dev wg0 proto kernel scope link src 10.13.13.1",
            "172.17.0.0/16 dev docker0 proto kernel scope link src 172.17.0.1 linkdown",
            "172.19.0.0/16 dev br-371961fae3f5 proto kernel scope link src 172.19.0.1",
        ])
        out = self._run_lan_selection(routes, default_dev="wg0")
        self.assertIn("MAIN=10.49.110.198", out)
        self.assertIn("NETS=10.49.110.0/24", out)
        self.assertNotIn("10.13.13", out.split("NETS=")[1])
        self.assertIn("default route is not a Wi-Fi/Ethernet interface", out)

    def test_lan_address_lists_every_lan_network(self) -> None:
        routes = "\n".join([
            "192.168.1.0/24 dev eth0 proto kernel scope link src 192.168.1.20",
            "10.42.0.0/24 dev wlan0 proto kernel scope link src 10.42.0.1",
        ])
        out = self._run_lan_selection(routes, default_dev="eth0")
        self.assertIn("MAIN=192.168.1.20", out)
        self.assertIn("OTHERS=10.42.0.1", out)
        self.assertIn("NETS=192.168.1.0/24 10.42.0.0/24", out)

    def test_firewall_rule_per_lan_network(self) -> None:
        text = SH.read_text(encoding="utf-8")
        self.assertIn('for net in "${LAN_SUBNETS[@]}"; do', text)
        # Recorded before adding, so --cleanup can remove it after a crash.
        self.assertLess(text.index('remember_fw ufw "$rule"'), text.index("sudo ufw allow $rule"))
        self.assertLess(text.index('remember_fw firewalld "$rule"'), text.index('--add-rich-rule="$rule"'))

    def test_vm_size_limits_match_between_apps(self) -> None:
        sh = SH.read_text(encoding="utf-8")
        ps1 = PS1.read_text(encoding="utf-8")
        for sh_name, ps_name, value in [
            ("MIN_VCPUS", "MinVcpus", "1"), ("MIN_MEM_GIB", "MinMemGib", "2"),
            ("MIN_DISK_GIB", "MinDiskGib", "6"), ("DEFAULT_MEM_GIB", "DefaultMemGib", "2"),
            ("DEFAULT_DISK_GIB", "DefaultDiskGib", "8"),
        ]:
            self.assertIn(f"{sh_name}={value}\n", sh)
            self.assertIn(f"$script:{ps_name} = {value}\n", ps1)
        for text in (sh, ps1):
            self.assertIn("LAN_TEST_DISK_GIB", text)
            self.assertIn("Enter", text)  # the size questions say Enter keeps the default
        self.assertIn('"${DISK_GIB}G"', sh)
        self.assertIn('"$($DiskGib)G"', ps1)
        self.assertIn("qemu-img resize", sh)
        self.assertIn("resize -q $overlay", ps1)

    def test_cleanup_modes_exist(self) -> None:
        sh = SH.read_text(encoding="utf-8")
        ps1 = PS1.read_text(encoding="utf-8")
        self.assertIn("--cleanup) ACTION=cleanup", sh)
        self.assertIn('"$KVM_STATE"', sh)
        self.assertIn("[switch]$Cleanup", ps1)
        self.assertIn('Add-Change "qemu-installed"', ps1)
        self.assertIn('Add-Change "whpx-enabled"', ps1)
        self.assertIn("Invoke-Cleanup", ps1)
        raw = (KIT / "lan-test-cleanup.cmd").read_bytes()
        self.assertIn(b"-Cleanup", raw)
        self.assertNotIn(b"\n", raw.replace(b"\r\n", b""))
        # sudo stays valid during the test so cleanup can remove the rules.
        self.assertIn("sudo -n -v", sh)

    def test_linux_cleanup_undoes_recorded_changes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            bin_dir = tmp / "bin"
            bin_dir.mkdir()
            calls = tmp / "calls.log"
            for name, body in {
                "sudo": 'shift_n() { [ "$1" = "-n" ] && shift; "$@"; }; shift_n "$@"',
                "ufw": f'echo "ufw $*" >> {calls}',
                "pacman": f'echo "pacman $*" >> {calls}',
                "gpasswd": f'echo "gpasswd $*" >> {calls}',
                "pkill": "exit 1",
            }.items():
                f = bin_dir / name
                f.write_text("#!/bin/sh\n" + body + "\n")
                f.chmod(0o755)
            state = tmp / "cache" / "dcl-lan-test"
            (state / "quick").mkdir(parents=True)
            (state / "quick" / "overlay.qcow2").write_text("disk")
            (state / "firewall-rules").write_text("ufw from 10.49.110.0/24 to any port 8443 proto tcp\n")
            (state / "installed-packages").write_text("qemu-base\ncurl\n")
            (state / "added-to-kvm-group").write_text("")
            env = dict(os.environ, PATH=f"{bin_dir}:/usr/bin:/bin", XDG_CACHE_HOME=str(tmp / "cache"),
                       USER="friend", TMPDIR=str(tmp))
            out = subprocess.run(["bash", str(SH), "--cleanup", "--yes"], env=env,
                                 capture_output=True, text=True)
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            log = calls.read_text()
            self.assertIn("ufw delete allow from 10.49.110.0/24 to any port 8443 proto tcp", log)
            self.assertIn("pacman -Rns --noconfirm curl qemu-base", log)
            self.assertIn("gpasswd -d friend kvm", log)
            self.assertFalse(state.exists(), "VM disk and state should be deleted")

    def test_guest_seed_fixes_paste_and_adds_bench(self) -> None:
        tpl = (KIT / "quick-user-data.in").read_text(encoding="utf-8")
        self.assertIn("path: /etc/inputrc", tpl)
        self.assertIn("set enable-bracketed-paste off", tpl)
        self.assertIn("path: /usr/local/bin/dcl-bench", tpl)
        # apt must not read the terminal, or it swallows pasted lines.
        self.assertIn("</dev/null", tpl)

    def test_windows_lan_addresses_skip_virtual_adapters(self) -> None:
        ps1 = PS1.read_text(encoding="utf-8")
        self.assertIn("Get-NetAdapter -Physical", ps1)
        self.assertIn("Wi-Fi Direct Virtual Adapter", ps1)
        self.assertNotIn("function Get-LanIPv4", ps1)

    def test_linux_matches_host_agent_kvm_argv(self) -> None:
        text = SH.read_text(encoding="utf-8")
        for flag in ("-enable-kvm", "-machine q35", "-cpu host", "if=virtio,format=raw",
                     "virtio-net-pci,netdev=net0", "-no-reboot"):
            self.assertIn(flag, text)

    def test_windows_matches_host_agent_whpx_argv(self) -> None:
        text = PS1.read_text(encoding="utf-8")
        # Disk is virtio on purpose (issue 9); the Host Agent still uses -hda.
        self.assertNotIn('"-hda"', text)
        self.assertIn("if=virtio,format=qcow2", text)
        self.assertIn("if=virtio,format=raw", text)
        for flag in ('"-accel", "whpx"', '"-M", "pc"', '"ich9-usb-ehci1"', '"usb-tablet"',
                     '"virtio-net-pci,netdev=net0"'):
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
        cleanup = sh[sh.index("\ncleanup() {"):sh.index("trap cleanup EXIT")]
        self.assertIn("remove_recorded_fw_rules", cleanup)
        self.assertIn('kill "$QEMU_PID"', cleanup)
        self.assertIn('rm -rf "$RUN_DIR"', cleanup)
        self.assertIn('kill "$SUDO_KEEPALIVE_PID"', cleanup)
        remover = sh[sh.index("remove_fw_rule() {"):sh.index("remove_recorded_fw_rules() {")]
        self.assertIn("ufw delete allow", remover)
        self.assertIn("--remove-rich-rule", remover)

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
                self.assertIn("dcl-lan-test-windows/lan-test-cleanup.cmd", names)
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
