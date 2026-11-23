"""Guest-image pipeline tests that do not need Packer, QEMU, or metal."""

from __future__ import annotations

import base64
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
PACKER = ROOT / "packer" / "dcl-guest.pkr.hcl"
VERSIONS = ROOT / "versions.env"
KASM = SCRIPTS / "provision-kasm.sh"
SUNSHINE = SCRIPTS / "provision-sunshine.sh"
BASE = SCRIPTS / "provision-base.sh"
JOIN = SCRIPTS / "dcl-guest-join.py"
JOIN_UNIT = SCRIPTS / "dcl-guest-join.service"
USER_DATA = ROOT / "cloud-init" / "user-data.pkrtpl"
SIGNER = SCRIPTS / "sign_image.py"

sys.path.insert(0, str(SCRIPTS))
import sign_image  # noqa: E402


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _load_versions() -> dict[str, str]:
    values: dict[str, str] = {}
    for line in _read(VERSIONS).splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key, value = stripped.split("=", 1)
        values[key] = value
    return values


class PackerTemplateTests(unittest.TestCase):
    def test_standard_and_pro_scripts_are_distinct(self) -> None:
        kasm = _read(KASM)
        sunshine = _read(SUNSHINE)
        self.assertIn("kasmvncserver.deb", kasm)
        self.assertIn("dcl-kasmvnc.service", kasm)
        self.assertIn("-disableBasicAuth", kasm)
        # LAN test: fw_cfg password turns KasmVNC basic auth on.
        self.assertIn("opt/org.dcl/lan-password", kasm)  # fallback
        self.assertIn("dmidecode -t 11", kasm)  # SMBIOS first
        self.assertIn("ExecStartPre=+/usr/local/sbin/dcl-kasmvnc-auth", kasm)
        self.assertIn("-u ubuntu -wo", kasm)
        # No LAN password: still create a user, or vncserver blocks on a prompt.
        self.assertIn("head -c 48 /dev/urandom", kasm)
        # No marker: vncserver blocks on a desktop-environment prompt.
        self.assertIn(".vnc/.de-was-selected", kasm)
        self.assertIn("websocket_port: 8443", kasm)
        self.assertIn("startxfce4", kasm)
        self.assertNotIn("sunshine-ubuntu-24.04", kasm)
        self.assertNotIn("kasm_release", sunshine)
        self.assertIn("sunshine-ubuntu-24.04", sunshine)
        self.assertIn("dpkg -i", sunshine)
        self.assertIn("app-dev.lizardbyte.app.Sunshine", sunshine)
        self.assertIn("Moonlight", sunshine)

    def test_packer_hcl_selects_kasm_vs_sunshine_by_lab_tier(self) -> None:
        hcl = _read(PACKER)
        self.assertIn('source "qemu" "ubuntu"', hcl)
        self.assertIn("disk_image", hcl)
        self.assertIn("provision-kasm.sh", hcl)
        self.assertIn("provision-sunshine.sh", hcl)
        self.assertIn('var.lab_tier == "standard"', hcl)
        self.assertIn('accelerator', hcl)
        self.assertNotIn('default     = "tcg"', hcl)
        self.assertNotIn("storj", hcl.lower())
        self.assertNotIn("tailscale.com/api", hcl)

    def test_base_uses_renterd_rclone_and_headscale_client(self) -> None:
        base = _read(BASE)
        self.assertIn("rclone", base)
        self.assertIn("tailscale", base)
        self.assertIn("not Tailscale SaaS", base)
        self.assertIn("not Storj", base)
        self.assertIn("cloud-init status --wait", base)
        self.assertNotRegex(base, r"(?m)^[^#]*tailscale up")

    def test_guest_join_unit_is_in_packer_and_does_not_advertise_exit(self) -> None:
        hcl = _read(PACKER)
        self.assertIn("dcl-guest-join.py", hcl)
        self.assertIn("dcl-guest-join.service", hcl)
        self.assertGreaterEqual(hcl.count("sudo -n {{ .Vars }} {{ .Path }}"), 3)
        join = _read(JOIN)
        unit = _read(JOIN_UNIT)
        self.assertIn("/v1/guest/bootstrap", join)
        self.assertIn("/v1/guest/saved-as-of", join)
        self.assertIn("dcl.matchmaker", join)
        self.assertIn("opt/org.dcl", join)
        self.assertIn("--s3-env-auth", join)
        self.assertIn("AWS_ACCESS_KEY_ID", join)
        self.assertIn("advertise-exit-node", join)
        self.assertIn("ExecStart=/usr/local/sbin/dcl-guest-join", unit)
        self.assertNotIn("storj", join.lower())

    def test_guest_join_redeems_bootstrap_without_live_tailscale(self) -> None:
        import http.server
        import socketserver
        import tempfile
        import threading

        captured: dict[str, object] = {}

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, format: str, *args: object) -> None:  # noqa: A003
                return

            def do_POST(self) -> None:  # noqa: N802
                length = int(self.headers.get("Content-Length", "0"))
                captured[self.path] = {
                    "authorization": self.headers.get("Authorization"),
                    "body": self.rfile.read(length).decode("utf-8"),
                }
                if self.path.endswith("/v1/guest/bootstrap"):
                    payload = json.dumps(
                        {
                            "loginServer": "http://headscale.test.invalid",
                            "preauthKey": "tskey-auth-test-guest",
                            "hostname": "dcl-guest-example",
                            "exitNode": "dcl-platform-exit",
                            "aclTags": ["tag:guest"],
                            "tailscaleUp": [
                                "up",
                                "--login-server",
                                "http://headscale.test.invalid",
                                "--authkey",
                                "tskey-auth-test-guest",
                                "--hostname",
                                "dcl-guest-example",
                                "--exit-node=dcl-platform-exit",
                            ],
                            "storage": {
                                "endpoint": "http://renterd.test.invalid",
                                "region": "us-east-1",
                                "bucket": "dcl-example",
                                "prefix": "files/",
                                "rcloneArgs": [
                                    "mount",
                                    ":s3:dcl-example/files",
                                    "/home/ubuntu/Persistent",
                                    "--s3-env-auth",
                                ],
                                "accessKeyId": "guest-access",
                                "secretAccessKey": "guest-secret-key",
                            },
                        }
                    ).encode()
                else:
                    payload = json.dumps({"sessionId": "11111111-1111-1111-1111-1111111111", "savedAsOf": "2026-09-11T21:00:00Z"}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

        with socketserver.TCPServer(("127.0.0.1", 0), Handler) as server:
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                origin = f"http://127.0.0.1:{server.server_address[1]}"
                with tempfile.TemporaryDirectory() as tmp:
                    cmdline = Path(tmp) / "cmdline"
                    saved = Path(tmp) / "saved-as-of"
                    cmdline.write_text(
                        f"console=ttyS0 dcl.matchmaker={origin} dcl.bootstrap=guest-token-value\n",
                        encoding="utf-8",
                    )
                    env = os.environ.copy()
                    env["DCL_CMDLINE_FILE"] = str(cmdline)
                    env["DCL_SAVED_AS_OF_PATH"] = str(saved)
                    env["DCL_GUEST_DRY_RUN"] = "1"
                    result = subprocess.run(
                        [sys.executable, str(JOIN)],
                        cwd=str(ROOT),
                        env=env,
                        capture_output=True,
                        text=True,
                        check=True,
                    )
                    self.assertIn("tailscale up", result.stdout)
                    self.assertIn("--authkey", result.stdout)
                    self.assertIn("rclone mount", result.stdout)
                    self.assertIn("--daemon", result.stdout)
                    self.assertNotIn("--advertise-exit-node", result.stdout)
                    self.assertTrue(saved.is_file())
            finally:
                server.shutdown()
        self.assertIn("/v1/guest/bootstrap", captured)
        self.assertEqual(
            captured["/v1/guest/bootstrap"]["authorization"],
            "Bearer guest-token-value",
        )

    def test_pinned_kasm_and_ubuntu_checksums(self) -> None:
        versions = _load_versions()
        self.assertEqual(len(versions["UBUNTU_CLOUDIMG_SHA256"]), 64)
        self.assertEqual(len(versions["KASM_VNC_DEB_SHA256"]), 64)
        self.assertIn(
            "kasmvncserver_noble_1.3.4_amd64.deb",
            versions["KASM_VNC_DEB_URL"],
        )
        self.assertIn("cloudimg-amd64.img", versions["UBUNTU_CLOUDIMG_URL"])
        # Dated folder: the bare release/ link moves and breaks the SHA-256.
        self.assertRegex(versions["UBUNTU_CLOUDIMG_URL"], r"/release-\d{8}/")
        self.assertIn("sunshine-ubuntu-24.04-amd64.deb", versions["SUNSHINE_DEB_URL"])

    def test_cloud_init_template_renders_ssh_key(self) -> None:
        template = _read(USER_DATA)
        self.assertIn("${ssh_public_key}", template)
        rendered = template.replace("${ssh_public_key}", "ssh-ed25519 AAAA-test")
        self.assertIn("ssh-ed25519 AAAA-test", rendered)
        self.assertNotIn("${ssh_public_key}", rendered)
        self.assertIn("NOPASSWD:ALL", rendered)

    def test_no_private_keys_in_templates(self) -> None:
        for path in (PACKER, USER_DATA, KASM, SUNSHINE, BASE, VERSIONS, JOIN, JOIN_UNIT):
            text = _read(path)
            self.assertNotIn("-----BEGIN", text)
            self.assertNotIn("PRIVATE KEY", text)


class SigningHelperTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmpdir = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmpdir.name)
        pem = self.dir / "dev.pem"
        subprocess.run(
            ["openssl", "genpkey", "-algorithm", "ED25519", "-out", str(pem)],
            check=True,
            capture_output=True,
        )
        self.pem = pem
        self.image = self.dir / "rootfs.img"
        self.image.write_bytes(b"dcl-fixture-standard-v1\n")

    def tearDown(self) -> None:
        self.tmpdir.cleanup()

    def test_checksum_sidecar_gnu_format(self) -> None:
        digest = sign_image.sha256_file(self.image).hex()
        sidecar = sign_image.write_checksum_file(self.image, digest)
        self.assertEqual(sidecar.name, "rootfs.img.sha256")
        self.assertEqual(sidecar.read_text(encoding="utf-8"), f"{digest}  rootfs.img\n")
        self.assertEqual(len(digest), 64)

    def test_sign_writes_checksum_signature_manifest_and_env(self) -> None:
        old_stdout, old_stderr = sys.stdout, sys.stderr
        sys.stdout, sys.stderr = io.StringIO(), io.StringIO()
        try:
            rc = sign_image.main(
                [
                    "sign",
                    "--image",
                    str(self.image),
                    "--key",
                    str(self.pem),
                    "--lab-tier",
                    "standard",
                ]
            )
        finally:
            sys.stdout, sys.stderr = old_stdout, old_stderr
        self.assertEqual(rc, 0)
        sig = (self.dir / "rootfs.img.sig").read_text(encoding="utf-8").strip()
        decoded = base64.b64decode(sig)
        self.assertEqual(len(decoded), 64)
        manifest = json.loads((self.dir / "rootfs.img.manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["labTier"], "standard")
        self.assertEqual(manifest["algorithm"], "ed25519")
        self.assertEqual(manifest["digestAlgorithm"], "sha256")
        self.assertEqual(manifest["signature"], sig)
        self.assertEqual(len(manifest["digestSha256"]), 64)
        size = self.image.stat().st_size
        self.assertEqual(manifest["sizeBytes"], size)
        snippet = (self.dir / "env.snippet").read_text(encoding="utf-8")
        self.assertIn(f"IMAGE_STANDARD_SIZE_BYTES={size}", snippet)
        self.assertIn("IMAGE_STANDARD_DIGEST=", snippet)
        self.assertIn("IMAGE_STANDARD_SIGNATURE=", snippet)
        self.assertIn("IMAGE_SIGNING_PUBLIC_KEY=", snippet)
        self.assertIn("IMAGE_LOCAL_DIGEST=", snippet)
        pub = sign_image.public_key_hex(self.pem)
        self.assertEqual(len(pub), 64)
        self.assertIn(pub, snippet)

    def test_pro_env_snippet_does_not_overwrite_local_digest(self) -> None:
        self.image.write_bytes(b"dcl-fixture-pro-v1\n")
        old_stdout, old_stderr = sys.stdout, sys.stderr
        sys.stdout, sys.stderr = io.StringIO(), io.StringIO()
        try:
            rc = sign_image.main(
                [
                    "sign",
                    "--image",
                    str(self.image),
                    "--key",
                    str(self.pem),
                    "--lab-tier",
                    "pro",
                ]
            )
        finally:
            sys.stdout, sys.stderr = old_stdout, old_stderr
        self.assertEqual(rc, 0)
        snippet = (self.dir / "env.snippet").read_text(encoding="utf-8")
        self.assertIn("IMAGE_PRO_DIGEST=", snippet)
        self.assertNotIn("IMAGE_LOCAL_DIGEST=", snippet)

    def test_sign_refuses_missing_key(self) -> None:
        env = os.environ.copy()
        env.pop("IMAGE_SIGNING_PRIVATE_KEY", None)
        result = subprocess.run(
            [sys.executable, str(SIGNER), "sign", "--image", str(self.image), "--lab-tier", "standard"],
            cwd=str(ROOT),
            env=env,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("IMAGE_SIGNING_PRIVATE_KEY", result.stderr)

    def test_checksum_cli_needs_no_key(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SIGNER), "checksum", "--image", str(self.image)],
            cwd=str(ROOT),
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(len(result.stdout.strip()), 64)


class GuestJoinBootstrapSourceTests(unittest.TestCase):
    """The stock Ubuntu cloud kernel has no qemu_fw_cfg module, so QEMU hosts
    hand the bootstrap to the guest as SMBIOS OEM strings (type 11)."""

    def _read_bootstrap(self, tmp: Path) -> str:
        env = os.environ.copy()
        env["DCL_CMDLINE_FILE"] = str(tmp / "cmdline")
        env["DCL_SMBIOS_DIR"] = str(tmp / "dmi")
        env["DCL_FWCFG_DIR"] = str(tmp / "fwcfg")
        env["DCL_GUEST_DRY_RUN"] = "1"
        code = (
            "import importlib.util, sys;"
            "spec = importlib.util.spec_from_file_location('join', sys.argv[1]);"
            "mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod);"
            "print(mod.read_bootstrap())"
        )
        return subprocess.run(
            [sys.executable, "-c", code, str(JOIN)],
            env=env,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()

    @staticmethod
    def _smbios_type11(strings: list[str]) -> bytes:
        header = bytes([11, 5, 0x00, 0x0B, len(strings)])
        return header + b"".join(s.encode() + b"\0" for s in strings) + b"\0"

    def test_reads_origin_and_token_from_smbios_oem_strings(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            tmp = Path(raw_tmp)
            entry = tmp / "dmi" / "11-0"
            entry.mkdir(parents=True)
            (entry / "raw").write_bytes(
                self._smbios_type11(
                    [
                        "dcl-lan-password=unrelated",
                        "dcl.matchmaker=https://mm.test/",
                        "dcl.bootstrap=guest-token-value",
                    ]
                )
            )
            self.assertEqual(
                self._read_bootstrap(tmp),
                "('https://mm.test', 'guest-token-value')",
            )

    def test_no_source_means_no_session(self) -> None:
        with tempfile.TemporaryDirectory() as raw_tmp:
            self.assertEqual(self._read_bootstrap(Path(raw_tmp)), "(None, None)")


if __name__ == "__main__":
    unittest.main()


class GuestJoinStorageEnvTests(unittest.TestCase):
    """Per-user storage keys are short-lived and come with a session token."""

    def _rclone_env(self, storage: dict, ambient_token: str | None = None) -> dict:
        import json

        env = os.environ.copy()
        env.pop("AWS_SESSION_TOKEN", None)
        if ambient_token is not None:
            env["AWS_SESSION_TOKEN"] = ambient_token
        code = (
            "import importlib.util, json, sys;"
            "spec = importlib.util.spec_from_file_location('join', sys.argv[1]);"
            "mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod);"
            "env = mod.rclone_env(json.loads(sys.argv[2]));"
            "print(json.dumps({k: env.get(k) for k in "
            "('AWS_ACCESS_KEY_ID', 'AWS_SECRET_ACCESS_KEY', 'AWS_SESSION_TOKEN')}))"
        )
        out = subprocess.run(
            [sys.executable, "-c", code, str(JOIN), json.dumps(storage)],
            env=env,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        return json.loads(out)

    def test_session_token_is_exported_for_rclone(self) -> None:
        env = self._rclone_env(
            {"accessKeyId": "temp-id", "secretAccessKey": "temp-secret", "sessionToken": "temp-token"}
        )
        self.assertEqual(env["AWS_ACCESS_KEY_ID"], "temp-id")
        self.assertEqual(env["AWS_SECRET_ACCESS_KEY"], "temp-secret")
        self.assertEqual(env["AWS_SESSION_TOKEN"], "temp-token")

    def test_a_stale_session_token_is_not_reused_with_static_keys(self) -> None:
        env = self._rclone_env(
            {"accessKeyId": "static-id", "secretAccessKey": "static-secret"},
            ambient_token="left-over",
        )
        self.assertIsNone(env["AWS_SESSION_TOKEN"])


def _join_call(code: str, env_extra: dict[str, str] | None = None) -> dict:
    """Load dcl-guest-join.py in a child process, run `code`, return its JSON."""
    env = os.environ.copy()
    env.update(env_extra or {})
    loader = (
        "import importlib.util, json, sys\n"
        "spec = importlib.util.spec_from_file_location('join', sys.argv[1])\n"
        "mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod)\n"
    )
    out = subprocess.run(
        [sys.executable, "-c", loader + code, str(JOIN)],
        env=env,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return json.loads(out)


class GuestShutdownFlushTests(unittest.TestCase):
    """At shutdown the guest waits for rclone to upload ~/Persistent (issue 21)."""

    def test_rclone_gets_a_loopback_remote_control_once(self) -> None:
        result = _join_call(
            "args = mod.rclone_argv({'rcloneArgs': ['mount', ':s3:b/p', '/mnt', '--s3-env-auth']})\n"
            "again = mod.rclone_argv({'rcloneArgs': args[1:]})\n"
            "print(json.dumps({'args': args, 'again': again}))\n"
        )
        args = result["args"]
        self.assertEqual(args.count("--rc"), 1)
        self.assertEqual(args[args.index("--rc-addr") + 1], "127.0.0.1:5572")
        self.assertEqual(result["again"], args)

    def test_waits_until_the_upload_queue_is_empty(self) -> None:
        result = _join_call(
            "answers = [{'diskCache': {'uploadsInProgress': 1, 'uploadsQueued': 2}},\n"
            "           {'diskCache': {'uploadsInProgress': 1, 'uploadsQueued': 0}},\n"
            "           {'diskCache': {'uploadsInProgress': 0, 'uploadsQueued': 0}}]\n"
            "calls = []\n"
            "def stats():\n"
            "    calls.append(1)\n"
            "    return answers[len(calls) - 1]\n"
            "done = mod.wait_for_uploads(stats=stats, sleep=lambda s: None, clock=lambda: 0)\n"
            "print(json.dumps({'done': done, 'calls': len(calls)}))\n"
        )
        self.assertEqual(result, {"done": True, "calls": 3})

    def test_gives_up_at_the_deadline_and_skips_when_rclone_is_absent(self) -> None:
        result = _join_call(
            "now = [0]\n"
            "def clock():\n"
            "    now[0] += 10\n"
            "    return now[0]\n"
            "busy = lambda: {'diskCache': {'uploadsQueued': 1}}\n"
            "late = mod.wait_for_uploads(stats=busy, sleep=lambda s: None, clock=clock)\n"
            "absent = mod.wait_for_uploads(stats=lambda: None, sleep=lambda s: None)\n"
            "print(json.dumps({'late': late, 'absent': absent,"
            " 'odd': mod.pending_uploads({'diskCache': 'x'})}))\n"
        )
        self.assertEqual(result, {"late": False, "absent": True, "odd": 0})

    def test_unit_gives_the_stop_step_time_to_upload(self) -> None:
        unit = (ROOT / "scripts" / "dcl-guest-join.service").read_text(encoding="utf-8")
        self.assertIn("ExecStop=/usr/local/sbin/dcl-guest-join --flush", unit)
        self.assertIn("TimeoutStopSec=85", unit)


class GuestDesktopPasswordTests(unittest.TestCase):
    """The desktop password is stored before the mesh join, readable by root only."""

    def test_password_file_is_private_and_single_line(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            result = _join_call(
                "import os, stat\n"
                "ok = mod.store_desktop_password({'desktopPassword': 'Abc123xyz'})\n"
                "p = mod.DESKTOP_PASSWORD_PATH\n"
                "print(json.dumps({'ok': ok, 'text': p.read_text(),"
                " 'mode': stat.S_IMODE(os.stat(p).st_mode)}))\n",
                {"DCL_RUN_DIR": tmp},
            )
        self.assertEqual(result, {"ok": True, "text": "Abc123xyz\n", "mode": 0o600})

    def test_missing_or_multiline_password_is_not_stored(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            result = _join_call(
                "missing = mod.store_desktop_password({})\n"
                "try:\n"
                "    mod.store_desktop_password({'desktopPassword': 'a\\nb'})\n"
                "    refused = False\n"
                "except RuntimeError:\n"
                "    refused = True\n"
                "print(json.dumps({'missing': missing, 'refused': refused,"
                " 'exists': mod.DESKTOP_PASSWORD_PATH.exists()}))\n",
                {"DCL_RUN_DIR": tmp},
            )
        self.assertEqual(result, {"missing": False, "refused": True, "exists": False})

    def test_bootstrap_is_retried_while_the_network_settles(self) -> None:
        result = _join_call(
            "import urllib.error\n"
            "calls = []; sleeps = []\n"
            "def post(url, token, timeout=None):\n"
            "    calls.append(url)\n"
            "    if len(calls) < 3:\n"
            "        raise urllib.error.URLError('network down')\n"
            "    return {'ok': True}\n"
            "body = mod.redeem_bootstrap('https://mm.example', 't', sleep=sleeps.append, post=post)\n"
            "print(json.dumps({'body': body, 'calls': len(calls), 'sleeps': sleeps,"
            " 'url': calls[0]}))\n"
        )
        self.assertEqual(result["body"], {"ok": True})
        self.assertEqual(result["calls"], 3)
        self.assertEqual(result["sleeps"], [3, 5])
        self.assertEqual(result["url"], "https://mm.example/v1/guest/bootstrap")

    def test_a_refused_token_is_final_and_gives_up_eventually(self) -> None:
        result = _join_call(
            "import urllib.error\n"
            "calls = []; sleeps = []\n"
            "def refused(url, token, timeout=None):\n"
            "    calls.append(1)\n"
            "    raise urllib.error.HTTPError(url, 404, 'gone', None, None)\n"
            "try:\n"
            "    mod.redeem_bootstrap('https://mm.example', 't', sleep=sleeps.append, post=refused)\n"
            "    first = 'returned'\n"
            "except urllib.error.HTTPError:\n"
            "    first = 'raised'\n"
            "refused_calls = len(calls)\n"
            "calls.clear()\n"
            "def down(url, token, timeout=None):\n"
            "    calls.append(1)\n"
            "    raise urllib.error.URLError('down')\n"
            "try:\n"
            "    mod.redeem_bootstrap('https://mm.example', 't', sleep=sleeps.append, post=down)\n"
            "    second = 'returned'\n"
            "except urllib.error.URLError:\n"
            "    second = 'raised'\n"
            "print(json.dumps({'first': first, 'refused_calls': refused_calls,"
            " 'second': second, 'down_calls': len(calls)}))\n"
        )
        self.assertEqual(
            result,
            {"first": "raised", "refused_calls": 1, "second": "raised", "down_calls": 7},
        )

    def test_mount_belongs_to_the_desktop_user(self) -> None:
        import pwd

        me = pwd.getpwuid(os.getuid())
        result = _join_call(
            "args = mod.rclone_argv({'rcloneArgs': ['mount', ':s3:b/p', '/mnt', '--s3-env-auth']})\n"
            "again = mod.rclone_argv({'rcloneArgs': args[1:]})\n"
            "print(json.dumps({'args': args, 'again': again,"
            " 'unknown': mod.mount_owner_args('no-such-user-dcl')}))\n",
            {"DCL_DESKTOP_USER": me.pw_name},
        )
        tail = ["--allow-other", "--uid", str(me.pw_uid), "--gid", str(me.pw_gid), "--umask", "077"]
        self.assertEqual(result["args"][-7:], tail)
        self.assertEqual(result["again"], result["args"])  # not added twice
        self.assertEqual(result["unknown"], [])

    def test_full_join_stores_the_password_before_tailscale_and_leaks_no_key(self) -> None:
        """Run the real script against a local Matchmaker stand-in.

        `tailscale` is a stub that records whether the password file already
        existed, then fails; the error message must not contain the mesh key.
        """
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
        from threading import Thread

        body = {
            "tailscaleUp": ["up", "--authkey", "tskey-auth-SECRETMESHKEY", "--hostname", "g"],
            "desktopPassword": "SessionDesktopPw42",
            "storage": {},
        }

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:  # noqa: N802
                payload = json.dumps(body).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def log_message(self, format: str, *args: object) -> None:
                return

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        Thread(target=server.serve_forever, daemon=True).start()
        try:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                run_dir = root / "run"
                (root / "cmdline").write_text(
                    f"dcl.matchmaker=http://127.0.0.1:{server.server_address[1]} "
                    "dcl.bootstrap=bootstrap-token-value\n"
                )
                bin_dir = root / "bin"
                bin_dir.mkdir()
                stub = bin_dir / "tailscale"
                stub.write_text(
                    "#!/bin/sh\n"
                    f'[ -s "{run_dir}/desktop-password" ] && echo yes >"{root}/seen" '
                    f'|| echo no >"{root}/seen"\n'
                    "exit 1\n"
                )
                stub.chmod(0o755)
                env = os.environ.copy()
                env.update(
                    {
                        "DCL_CMDLINE_FILE": str(root / "cmdline"),
                        "DCL_SMBIOS_DIR": str(root / "dmi"),
                        "DCL_FWCFG_DIR": str(root / "fwcfg"),
                        "DCL_RUN_DIR": str(run_dir),
                        "PATH": f"{bin_dir}{os.pathsep}{env['PATH']}",
                    }
                )
                done = subprocess.run(
                    [sys.executable, str(JOIN)], env=env, capture_output=True, text=True
                )
                self.assertEqual(done.returncode, 1)
                self.assertEqual((root / "seen").read_text().strip(), "yes")
                self.assertTrue((run_dir / "session-vm").exists())
                self.assertEqual(
                    (run_dir / "desktop-password").read_text(), "SessionDesktopPw42\n"
                )
                self.assertIn("tailscale exited with status 1", done.stderr)
                self.assertNotIn("SECRETMESHKEY", done.stderr + done.stdout)
                self.assertNotIn("SessionDesktopPw42", done.stderr + done.stdout)
        finally:
            server.shutdown()

    def test_desktop_script_covers_session_locked_lan_and_open(self) -> None:
        kasm = _read(KASM)
        self.assertIn("SESSION_PASSWORD=/run/dcl/desktop-password", kasm)
        self.assertIn("SESSION_MARK=/run/dcl/session-vm", kasm)
        self.assertIn("mode=session", kasm)
        self.assertIn("mode=locked", kasm)
        self.assertIn('if [ "$mode" != open ]; then', kasm)
        # The desktop starts after the join, which stores the password.
        self.assertIn("After=network-online.target dcl-guest-join.service", kasm)

    def test_base_image_has_fusermount_and_plain_paste(self) -> None:
        base = _read(BASE)
        self.assertIn("/usr/local/bin/fusermount", base)
        self.assertIn("set enable-bracketed-paste off", base)
