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
        self.assertIn("opt/org.dcl/lan-password", kasm)
        self.assertIn("ExecStartPre=+/usr/local/sbin/dcl-kasmvnc-auth", kasm)
        self.assertIn("-u ubuntu -wo", kasm)
        # No LAN password: still create a user, or vncserver blocks on a prompt.
        self.assertIn("head -c 48 /dev/urandom", kasm)
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
        snippet = (self.dir / "env.snippet").read_text(encoding="utf-8")
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


class ExtractKernelTests(unittest.TestCase):
    def test_extract_kernel_refuses_to_guess(self) -> None:
        script = SCRIPTS / "extract_kernel.sh"
        result = subprocess.run(
            ["bash", str(script), "/tmp/missing.img", "/tmp/vmlinux"],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("FIRECRACKER_KERNEL", result.stderr)
        self.assertIn("extract-vmlinux", _read(script))


if __name__ == "__main__":
    unittest.main()
