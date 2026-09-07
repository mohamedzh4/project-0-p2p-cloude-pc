#!/usr/bin/env python3
"""Sign guest disks the way Host Agent `image.rs` verifies them.

The platform signs the **raw 32-byte SHA-256** of the image with Ed25519.
The signature is standard base64. The public key is 64 hex chars
(`IMAGE_SIGNING_PUBLIC_KEY`). Matchmaker advertises digest + signature only.

Private keys stay in the operator env (`IMAGE_SIGNING_PRIVATE_KEY` = PEM path).
Never commit PEMs, `.env`, or the image bytes.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ALG = "ed25519"
DIGEST_ALG = "sha256"


def sha256_file(path: Path) -> bytes:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.digest()


def sha256_bytes(data: bytes) -> bytes:
    return hashlib.sha256(data).digest()


def checksum_line(digest_hex: str, image_path: Path) -> str:
    return f"{digest_hex}  {image_path.name}\n"


def write_checksum_file(image_path: Path, digest_hex: str) -> Path:
    sidecar = Path(str(image_path) + ".sha256")
    sidecar.write_text(checksum_line(digest_hex, image_path), encoding="utf-8")
    return sidecar


def _run_openssl(args: list[str], stdin: bytes | None = None) -> bytes:
    result = subprocess.run(
        ["openssl", *args],
        input=stdin,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"openssl {' '.join(args)} failed: {detail}")
    return result.stdout


def public_key_hex(private_key_pem: Path) -> str:
    der = _run_openssl(
        ["pkey", "-in", str(private_key_pem), "-pubout", "-outform", "DER"]
    )
    if len(der) < 32:
        raise RuntimeError("openssl did not return an Ed25519 public key")
    return der[-32:].hex()


def sign_digest(private_key_pem: Path, digest: bytes) -> bytes:
    if len(digest) != 32:
        raise RuntimeError("Ed25519 image signature is over the raw 32-byte SHA-256")
    with tempfile.NamedTemporaryFile(prefix="dcl-digest-", delete=False) as handle:
        handle.write(digest)
        digest_path = Path(handle.name)
    try:
        signature = _run_openssl(
            [
                "pkeyutl",
                "-sign",
                "-inkey",
                str(private_key_pem),
                "-rawin",
                "-in",
                str(digest_path),
            ]
        )
    finally:
        digest_path.unlink(missing_ok=True)
    if len(signature) != 64:
        raise RuntimeError(
            f"Ed25519 signature must be 64 bytes, openssl returned {len(signature)}"
        )
    return signature


def verify_digest(private_key_pem: Path, digest: bytes, signature: bytes) -> None:
    pub_pem = _run_openssl(["pkey", "-in", str(private_key_pem), "-pubout"])
    with tempfile.NamedTemporaryFile(prefix="dcl-pub-", suffix=".pem", delete=False) as pub:
        pub.write(pub_pem)
        pub_path = Path(pub.name)
    with tempfile.NamedTemporaryFile(prefix="dcl-digest-", delete=False) as dig:
        dig.write(digest)
        digest_path = Path(dig.name)
    with tempfile.NamedTemporaryFile(prefix="dcl-sig-", delete=False) as sig:
        sig.write(signature)
        sig_path = Path(sig.name)
    try:
        _run_openssl(
            [
                "pkeyutl",
                "-verify",
                "-pubin",
                "-inkey",
                str(pub_path),
                "-rawin",
                "-in",
                str(digest_path),
                "-sigfile",
                str(sig_path),
            ]
        )
    finally:
        pub_path.unlink(missing_ok=True)
        digest_path.unlink(missing_ok=True)
        sig_path.unlink(missing_ok=True)


def manifest_body(
    *,
    lab_tier: str,
    digest_hex: str,
    signature_b64: str,
    image_file: str,
) -> dict[str, str]:
    return {
        "labTier": lab_tier,
        "digestSha256": digest_hex,
        "signature": signature_b64,
        "algorithm": ALG,
        "digestAlgorithm": DIGEST_ALG,
        "imageFile": image_file,
    }


def env_snippet(
    *,
    lab_tier: str,
    digest_hex: str,
    signature_b64: str,
    public_hex: str,
) -> str:
    tier = lab_tier.upper()
    lines = [
        f"IMAGE_{tier}_DIGEST={digest_hex}",
        f"IMAGE_{tier}_SIGNATURE={signature_b64}",
        f"IMAGE_SIGNING_PUBLIC_KEY={public_hex}",
    ]
    if lab_tier == "standard":
        lines.append(f"IMAGE_LOCAL_DIGEST={digest_hex}")
    return "\n".join(lines) + "\n"


def require_key_path(value: str | None) -> Path:
    raw = (value or os.environ.get("IMAGE_SIGNING_PRIVATE_KEY") or "").strip()
    if not raw:
        raise RuntimeError(
            "IMAGE_SIGNING_PRIVATE_KEY is required (path to an Ed25519 PEM). "
            "Generate with: openssl genpkey -algorithm ED25519 -out platform-ed25519.pem"
        )
    path = Path(raw).expanduser()
    if not path.is_file():
        raise RuntimeError(f"signing key not found: {path}")
    return path


def cmd_checksum(args: argparse.Namespace) -> int:
    image = Path(args.image)
    digest_hex = sha256_file(image).hex()
    sidecar = write_checksum_file(image, digest_hex)
    print(digest_hex)
    print(sidecar, file=sys.stderr)
    return 0


def cmd_public_key(args: argparse.Namespace) -> int:
    key = require_key_path(args.key)
    print(public_key_hex(key))
    return 0


def cmd_sign(args: argparse.Namespace) -> int:
    image = Path(args.image)
    key = require_key_path(args.key)
    lab_tier = args.lab_tier
    digest = sha256_file(image)
    digest_hex = digest.hex()
    signature = sign_digest(key, digest)
    if args.verify:
        verify_digest(key, digest, signature)
    signature_b64 = base64.b64encode(signature).decode("ascii")
    checksum_path = write_checksum_file(image, digest_hex)
    sig_path = Path(str(image) + ".sig")
    sig_path.write_text(signature_b64 + "\n", encoding="utf-8")
    body = manifest_body(
        lab_tier=lab_tier,
        digest_hex=digest_hex,
        signature_b64=signature_b64,
        image_file=image.name,
    )
    manifest_path = Path(str(image) + ".manifest.json")
    manifest_path.write_text(json.dumps(body, indent=2) + "\n", encoding="utf-8")
    snippet = env_snippet(
        lab_tier=lab_tier,
        digest_hex=digest_hex,
        signature_b64=signature_b64,
        public_hex=public_key_hex(key),
    )
    snippet_path = image.with_name("env.snippet")
    snippet_path.write_text(snippet, encoding="utf-8")
    print(json.dumps(body, indent=2))
    print(checksum_path, file=sys.stderr)
    print(sig_path, file=sys.stderr)
    print(manifest_path, file=sys.stderr)
    print(snippet_path, file=sys.stderr)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    checksum = sub.add_parser("checksum", help="Write IMAGE.sha256 (no private key).")
    checksum.add_argument("--image", required=True)
    checksum.set_defaults(func=cmd_checksum)

    pub = sub.add_parser("public-key", help="Print IMAGE_SIGNING_PUBLIC_KEY hex.")
    pub.add_argument("--key", default=None, help="PEM path; else IMAGE_SIGNING_PRIVATE_KEY")
    pub.set_defaults(func=cmd_public_key)

    sign = sub.add_parser("sign", help="Checksum + Ed25519 sidecar files.")
    sign.add_argument("--image", required=True)
    sign.add_argument("--key", default=None, help="PEM path; else IMAGE_SIGNING_PRIVATE_KEY")
    sign.add_argument("--lab-tier", required=True, choices=("standard", "pro"))
    sign.add_argument("--verify", action="store_true", default=True)
    sign.add_argument("--no-verify", action="store_false", dest="verify")
    sign.set_defaults(func=cmd_sign)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except Exception as exc:  # noqa: BLE001 — CLI surface
        print(f"sign_image: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
