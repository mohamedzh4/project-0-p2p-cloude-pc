"""Pro gaming image: virtual monitor, display setup, stream token, provisioning."""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parent
SCRIPTS = ROOT / "scripts"
PACKER = ROOT / "packer" / "dcl-guest.pkr.hcl"
GAMING = SCRIPTS / "provision-gaming.sh"
STREAM = SCRIPTS / "provision-stream.sh"
UNIT = SCRIPTS / "dcl-stream.service"
WAIT = SCRIPTS / "dcl-stream-wait"
JOIN = SCRIPTS / "dcl-guest-join.py"
BUILD = SCRIPTS / "build.sh"

sys.path.insert(0, str(SCRIPTS))
import make_edid  # noqa: E402

_spec = __import__("importlib.util").util.spec_from_file_location("display", SCRIPTS / "dcl-display-setup.py")
display = __import__("importlib.util").util.module_from_spec(_spec)
_spec.loader.exec_module(display)


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def _decode_dtd(d: bytes) -> tuple[int, int, float]:
    pclk = (d[0] | d[1] << 8) * 10_000
    h_act = d[2] | (d[4] >> 4) << 8
    h_blank = d[3] | (d[4] & 0xF) << 8
    v_act = d[5] | (d[7] >> 4) << 8
    v_blank = d[6] | (d[7] & 0xF) << 8
    return h_act, v_act, pclk / ((h_act + h_blank) * (v_act + v_blank))


class EdidTests(unittest.TestCase):
    def test_block_is_valid_edid_1_3(self) -> None:
        e = make_edid.edid()
        self.assertEqual(len(e), 128)
        self.assertEqual(e[:8], b"\x00\xff\xff\xff\xff\xff\xff\x00")
        self.assertEqual(sum(e) % 256, 0, "checksum")
        self.assertEqual((e[18], e[19]), (1, 3))
        maker = int.from_bytes(e[8:10], "big")
        letters = "".join(chr(((maker >> s) & 0x1F) + 64) for s in (10, 5, 0))
        self.assertEqual(letters, "DCL")
        self.assertEqual(e[126], 0, "no extension blocks")

    def test_modes_are_what_games_need(self) -> None:
        e = make_edid.edid()
        modes = [_decode_dtd(e[54 + 18 * i : 72 + 18 * i]) for i in range(3)]
        self.assertEqual([(w, h) for w, h, _ in modes], [(1920, 1080), (1920, 1080), (2560, 1440)])
        self.assertAlmostEqual(modes[0][2], 60.0, places=1)
        self.assertAlmostEqual(modes[1][2], 120.0, places=1)
        self.assertAlmostEqual(modes[2][2], 59.95, places=1)
        self.assertIn(b"DCL Stream", e[108:126])

    def test_cli_writes_the_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "e.bin"
            subprocess.run([sys.executable, str(SCRIPTS / "make_edid.py"), str(out)], check=True)
            self.assertEqual(out.read_bytes(), make_edid.edid())


def _gpu(root: Path, addr: str, vendor: str, device: str, connectors: dict[str, str] | None = None, card: str = "card1") -> None:
    dev = root / "sys/bus/pci/devices" / addr
    dev.mkdir(parents=True)
    (dev / "class").write_text("0x030000\n")
    (dev / "vendor").write_text(vendor + "\n")
    (dev / "device").write_text(device + "\n")
    if connectors is None:
        return
    drm = root / "sys/class/drm"
    (drm / card).mkdir(parents=True, exist_ok=True)
    (drm / card / "device").symlink_to(dev)
    for name, status in connectors.items():
        c = drm / f"{card}-{name}"
        c.mkdir()
        (c / "status").write_text(status + "\n")
        (root / "sys/kernel/debug/dri" / card.removeprefix("card") / name).mkdir(parents=True)


class DisplaySetupTests(unittest.TestCase):
    def run_setup(self, build) -> tuple[str | None, Path]:
        tmp = tempfile.mkdtemp()
        root = Path(tmp)
        build(root)
        edid = root / "edid.bin"
        edid.write_bytes(make_edid.edid())
        display.EDID = edid
        display.main(root)
        conf = root / display.XORG_CONF
        return (conf.read_text() if conf.exists() else None), root

    def test_nvidia_without_a_monitor_gets_a_virtual_one(self) -> None:
        def build(r: Path) -> None:
            _gpu(r, "0000:00:01.0", "0x1234", "0x1111")  # QEMU VGA: ignored
            _gpu(r, "0000:01:00.0", "0x10de", "0x2786", {"DP-1": "disconnected"})

        conf, _ = self.run_setup(build)
        self.assertIn('Driver "nvidia"', conf)
        self.assertIn('BusID "PCI:1:0:0"', conf)
        self.assertIn('"ConnectedMonitor" "DFP-0"', conf)
        self.assertIn("CustomEDID", conf)
        self.assertIn('Section "ServerLayout"', conf)

    def test_a_real_monitor_or_dummy_plug_is_left_alone(self) -> None:
        def build(r: Path) -> None:
            _gpu(r, "0000:01:00.0", "0x10de", "0x2786", {"HDMI-A-1": "connected"})

        conf, _ = self.run_setup(build)
        self.assertIn('BusID "PCI:1:0:0"', conf)
        self.assertNotIn("ConnectedMonitor", conf)

    def test_amd_gets_an_edid_override_through_debugfs(self) -> None:
        def build(r: Path) -> None:
            _gpu(r, "0000:06:00.0", "0x1002", "0x744c", {"DP-1": "disconnected", "HDMI-A-1": "disconnected"})

        conf, root = self.run_setup(build)
        self.assertIn('Driver "amdgpu"', conf)
        self.assertIn('BusID "PCI:6:0:0"', conf)
        self.assertNotIn("CustomEDID", conf)
        dbg = root / "sys/kernel/debug/dri/1/DP-1"
        self.assertEqual((dbg / "edid_override").read_bytes(), make_edid.edid())
        self.assertEqual((dbg / "force").read_text(), "on\n")
        self.assertFalse((root / "sys/kernel/debug/dri/1/HDMI-A-1/force").exists(), "only one connector")

    def test_no_passthrough_gpu_writes_nothing(self) -> None:
        conf, _ = self.run_setup(lambda r: _gpu(r, "0000:00:01.0", "0x1234", "0x1111"))
        self.assertIsNone(conf)

    def test_nvidia_wins_over_an_igpu(self) -> None:
        def build(r: Path) -> None:
            _gpu(r, "0000:00:02.0", "0x8086", "0x4680")
            _gpu(r, "0000:01:00.0", "0x10de", "0x2786")

        self.assertEqual(display.find_gpus(Path(tempfile.mkdtemp())), [])
        root = Path(tempfile.mkdtemp())
        build(root)
        self.assertEqual(display.find_gpus(root)[0], ("0000:01:00.0", "nvidia"))

    def test_bus_id_is_decimal(self) -> None:
        self.assertEqual(display.bus_id("0000:0a:1f.3"), "PCI:10:31:3")


def _join(code: str, env: dict[str, str]) -> dict:
    loader = (
        "import importlib.util, json, sys\n"
        "spec = importlib.util.spec_from_file_location('join', sys.argv[1])\n"
        "mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod)\n"
    )
    out = subprocess.run(
        [sys.executable, "-c", loader + code, str(JOIN)],
        env={**os.environ, **env},
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return json.loads(out)


class StreamTokenTests(unittest.TestCase):
    TOKEN = "ab" * 32

    def test_token_file_is_private(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            r = _join(
                "import os, stat\n"
                f"ok = mod.store_stream_token('{self.TOKEN.upper()}')\n"
                "p = mod.STREAM_TOKEN_PATH\n"
                "print(json.dumps({'ok': ok, 'text': p.read_text(), 'mode': stat.S_IMODE(os.stat(p).st_mode)}))\n",
                {"DCL_RUN_DIR": tmp},
            )
        self.assertEqual(r, {"ok": True, "text": self.TOKEN + "\n", "mode": 0o600})

    def test_bad_or_missing_tokens(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            r = _join(
                "missing = mod.store_stream_token(None)\n"
                "try:\n"
                "    mod.store_stream_token('xyz')\n"
                "    refused = False\n"
                "except RuntimeError:\n"
                "    refused = True\n"
                "print(json.dumps({'missing': missing, 'refused': refused, 'exists': mod.STREAM_TOKEN_PATH.exists()}))\n",
                {"DCL_RUN_DIR": tmp},
            )
        self.assertEqual(r, {"missing": False, "refused": True, "exists": False})

    def test_manual_token_from_the_kernel_command_line(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cmd = Path(tmp) / "cmdline"
            cmd.write_text(f"root=/dev/vda1 dcl.stream_token={self.TOKEN} quiet\n")
            r = _join(
                "print(json.dumps({'t': mod.read_manual_stream_token()}))\n",
                {"DCL_CMDLINE_FILE": str(cmd), "DCL_SMBIOS_DIR": tmp},
            )
        self.assertEqual(r, {"t": self.TOKEN})

    def test_bootstrap_token_is_stored_before_the_mesh_join(self) -> None:
        src = _read(JOIN)
        main = src[src.index("def main(") :]
        self.assertLess(main.index("store_stream_token(body.get(\"streamToken\"))"), main.index("_run(join_from_bootstrap(body))"))


class ProvisioningTests(unittest.TestCase):
    def test_pro_steps_run_only_for_pro(self) -> None:
        hcl = _read(PACKER)
        self.assertIn('is_pro          = var.lab_tier == "pro"', hcl)
        self.assertIn("provision-gaming.sh", hcl)
        self.assertIn("provision-stream.sh", hcl)
        self.assertIn(": [local.skip]", hcl)
        for script in (GAMING, STREAM):
            self.assertIn('if [[ "${LAB_TIER:-}" != "pro" ]]', _read(script))

    def test_gaming_stack(self) -> None:
        s = _read(GAMING)
        for needed in (
            "dpkg --add-architecture i386",
            "steam-installer",
            "wine64 wine32:i386",
            "lutris",
            "gamemode mangohud",
            "libnvidia-gl-${nv_ver}:i386",
            "libnvidia-encode-${nv_ver}",
            "mesa-vulkan-drivers:i386",
            "mesa-va-drivers",
            "autologin-user=ubuntu",
            "support.null-audio-sink",
            "Before=display-manager.service",
            "nvidia-drm.modeset=1",
        ):
            self.assertIn(needed, s, needed)

    def test_streamer_records_the_game_sink(self) -> None:
        sink = re.search(r'node.name\s*=\s*"([^"]+)"', _read(GAMING)).group(1)
        self.assertIn(f"DCL_STREAM_AUDIO={sink}.monitor", _read(UNIT))

    def test_streamer_build_and_service(self) -> None:
        s = _read(STREAM)
        self.assertIn("cargo build --release --locked -p dcl-stream-server --features ffmpeg,uinput", s)
        self.assertIn('rm -rf /opt/rustup /opt/cargo "$SRC"', s, "toolchain removed from the image")
        self.assertIn('GROUP="input", MODE="0660"', s)
        unit = _read(UNIT)
        self.assertIn("User=ubuntu", unit)
        self.assertNotIn("User=root", unit)
        self.assertIn("ExecStartPre=/usr/local/libexec/dcl-stream-wait", unit)
        self.assertTrue((REPO / "stream" / "Cargo.lock").is_file(), "--locked needs the lock file")
        subprocess.run(["sh", "-n", str(WAIT)], check=True)

    def test_build_passes_the_new_variables(self) -> None:
        b = _read(BUILD)
        for v in ("nvidia_driver_package", "rust_toolchain", "stream_source_dir"):
            self.assertIn(f'-var "{v}=', b)
        self.assertIn("--exclude=./target", b)
        versions = _read(ROOT / "versions.env")
        rust = re.search(r"^RUST_TOOLCHAIN=(\S+)", versions, re.M).group(1)
        need = re.search(r'rust-version = "([^"]+)"', _read(REPO / "stream" / "Cargo.toml")).group(1)
        self.assertGreaterEqual(tuple(map(int, rust.split("."))), tuple(map(int, need.split("."))))


if __name__ == "__main__":
    unittest.main()
