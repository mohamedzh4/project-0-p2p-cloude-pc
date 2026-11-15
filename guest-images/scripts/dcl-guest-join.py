#!/usr/bin/env python3
"""Guest join: redeem Matchmaker bootstrap, tailscale up, rclone mount, FR6.

Reads dcl.matchmaker= and dcl.bootstrap= from, in order: /proc/cmdline,
SMBIOS OEM strings (QEMU -smbios type=11), then QEMU fw_cfg
opt/org.dcl/{matchmaker,bootstrap}. SMBIOS is the path that works on the
stock Ubuntu cloud kernel, which ships no qemu_fw_cfg module. Never
--advertise-exit-node or --advertise-tags. S3 keys stay in env (--s3-env-auth).
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

CMDLINE_FILE = os.environ.get("DCL_CMDLINE_FILE", "/proc/cmdline")
FWCFG_DIR = os.environ.get(
    "DCL_FWCFG_DIR",
    "/sys/firmware/qemu_fw_cfg/by_name",
)
SMBIOS_DIR = os.environ.get("DCL_SMBIOS_DIR", "/sys/firmware/dmi/entries")
SAVED_PATH = Path(os.environ.get("DCL_SAVED_AS_OF_PATH", "/var/lib/dcl/saved-as-of"))
MOUNT_POINT = os.environ.get("DCL_GUEST_MOUNT", "/home/ubuntu/Persistent")
DRY_RUN = os.environ.get("DCL_GUEST_DRY_RUN", "") == "1"


def _read_text(path: Path) -> str | None:
    try:
        return path.read_text(encoding="utf-8").strip()
    except OSError:
        return None


def _cmdline_value(cmdline: str, key: str) -> str | None:
    prefix = key + "="
    for token in cmdline.split():
        if token.startswith(prefix):
            value = token[len(prefix) :].strip()
            return value or None
    return None


def _smbios_oem_strings() -> list[str]:
    """OEM strings (SMBIOS type 11). Sysfs first, then dmidecode. Root-only."""
    strings: list[str] = []
    for entry in sorted(Path(SMBIOS_DIR).glob("11-*")):
        try:
            raw = (entry / "raw").read_bytes()
        except OSError:
            continue
        if len(raw) < 2:
            continue
        # raw[1] is the formatted-area length; NUL-terminated strings follow.
        for chunk in raw[raw[1] :].split(b"\0"):
            if chunk:
                strings.append(chunk.decode("utf-8", "replace").strip())
    if strings or DRY_RUN:
        return strings
    try:
        out = subprocess.run(
            ["dmidecode", "-t", "11"], capture_output=True, text=True, check=False
        ).stdout
    except OSError:
        return strings
    for line in out.splitlines():
        head, sep, value = line.strip().partition(": ")
        if sep and head.startswith("String "):
            strings.append(value.strip())
    return strings


def _oem_value(strings: list[str], key: str) -> str | None:
    prefix = key + "="
    for item in strings:
        if item.startswith(prefix):
            return item[len(prefix) :].strip() or None
    return None


def read_bootstrap() -> tuple[str | None, str | None]:
    cmdline = _read_text(Path(CMDLINE_FILE)) or ""
    origin = _cmdline_value(cmdline, "dcl.matchmaker")
    token = _cmdline_value(cmdline, "dcl.bootstrap")
    if not origin or not token:
        oem = _smbios_oem_strings()
        origin = origin or _oem_value(oem, "dcl.matchmaker")
        token = token or _oem_value(oem, "dcl.bootstrap")
    fw = Path(FWCFG_DIR)
    if not origin:
        origin = _read_text(fw / "opt" / "org.dcl" / "matchmaker" / "raw")
    if not token:
        token = _read_text(fw / "opt" / "org.dcl" / "bootstrap" / "raw")
    if origin:
        origin = origin.rstrip("/")
    return origin, token


def _json_post(url: str, token: str, payload: dict | None = None) -> dict:
    data = None if payload is None else json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=data,
        method="POST",
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/json",
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        raw = response.read()
    if not raw:
        return {}
    body = json.loads(raw.decode("utf-8"))
    if not isinstance(body, dict):
        raise RuntimeError("guest bootstrap response was not an object")
    return body


def report_saved_as_of(origin: str, token: str) -> str:
    stamp = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    SAVED_PATH.parent.mkdir(parents=True, exist_ok=True)
    SAVED_PATH.write_text(stamp + "\n", encoding="utf-8")
    if DRY_RUN:
        return stamp
    _json_post(f"{origin}/v1/guest/saved-as-of", token, {"savedAsOf": stamp})
    return stamp


def join_from_bootstrap(body: dict) -> list[str]:
    args = body.get("tailscaleUp")
    if not isinstance(args, list) or not args:
        raise RuntimeError("bootstrap response missing tailscaleUp")
    argv = ["tailscale"] + [str(item) for item in args]
    joined = " ".join(argv)
    if "--advertise-exit-node" in joined or "--advertise-tags" in joined:
        raise RuntimeError("guest join must not advertise exit-node or tags")
    return argv


def rclone_env(storage: dict) -> dict[str, str]:
    env = os.environ.copy()
    access = storage.get("accessKeyId")
    secret = storage.get("secretAccessKey")
    if not isinstance(access, str) or not isinstance(secret, str):
        raise RuntimeError("bootstrap storage is missing S3 env keys")
    env["AWS_ACCESS_KEY_ID"] = access
    env["AWS_SECRET_ACCESS_KEY"] = secret
    return env


def rclone_argv(storage: dict) -> list[str]:
    args = storage.get("rcloneArgs")
    if not isinstance(args, list) or not args:
        raise RuntimeError("bootstrap storage missing rcloneArgs")
    argv = ["rclone"] + [str(item) for item in args]
    if "--s3-env-auth" not in argv:
        raise RuntimeError("guest rclone must use --s3-env-auth")
    return argv


def _run(argv: list[str], env: dict[str, str] | None = None) -> None:
    if DRY_RUN:
        print(" ".join(argv))
        return
    subprocess.run(argv, check=True, env=env)


def main(argv: list[str]) -> int:
    flush_only = "--flush" in argv
    origin, token = read_bootstrap()
    if not origin or not token:
        # Image can boot without a session (packer, Hyper-V leftover).
        return 0
    if flush_only:
        try:
            report_saved_as_of(origin, token)
        except (urllib.error.URLError, RuntimeError, json.JSONDecodeError):
            return 1
        return 0

    try:
        body = _json_post(f"{origin}/v1/guest/bootstrap", token)
        _run(join_from_bootstrap(body))
        storage = body.get("storage")
        if not isinstance(storage, dict):
            raise RuntimeError("bootstrap response missing storage")
        if not DRY_RUN:
            Path(MOUNT_POINT).mkdir(parents=True, exist_ok=True)
        rclone = rclone_argv(storage)
        if "--daemon" not in rclone:
            rclone.append("--daemon")
        _run(rclone, env=rclone_env(storage))
        report_saved_as_of(origin, token)
    except (urllib.error.URLError, RuntimeError, json.JSONDecodeError, subprocess.CalledProcessError):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
