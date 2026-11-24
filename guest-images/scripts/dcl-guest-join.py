#!/usr/bin/env python3
"""Guest join: redeem Matchmaker bootstrap, tailscale up, rclone mount, FR6.

Order matters. The desktop password is stored before the guest joins the
mesh, so the desktop is never reachable on the mesh without it.

Reads dcl.matchmaker= and dcl.bootstrap= from, in order: /proc/cmdline,
SMBIOS OEM strings (QEMU -smbios type=11), then QEMU fw_cfg
opt/org.dcl/{matchmaker,bootstrap}. SMBIOS is the path that works on the
stock Ubuntu cloud kernel, which ships no qemu_fw_cfg module. Never
--advertise-exit-node or --advertise-tags. S3 keys stay in env (--s3-env-auth).
"""

from __future__ import annotations

import json
import os
import pwd
import subprocess
import sys
import time
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
# Read by dcl-kasmvnc-auth (root) before the desktop starts.
RUN_DIR = Path(os.environ.get("DCL_RUN_DIR", "/run/dcl"))
DESKTOP_PASSWORD_PATH = RUN_DIR / "desktop-password"
# Read by dcl-stream-server (runs as the desktop user) on Pro guests.
STREAM_TOKEN_PATH = RUN_DIR / "stream-token"
SESSION_MARK_PATH = RUN_DIR / "session-vm"
DESKTOP_USER = os.environ.get("DCL_DESKTOP_USER", "ubuntu")
# The network can still be settling when the unit starts. Seconds to wait
# between attempts to reach the Matchmaker.
BOOTSTRAP_RETRY_DELAYS = (3, 5, 8, 12, 15, 20)
BOOTSTRAP_TIMEOUT_SECONDS = 15
# rclone's remote control, loopback only, so shutdown can wait for uploads.
RC_ADDR = "127.0.0.1:5572"
RC_ARGS = ["--rc", "--rc-addr", RC_ADDR, "--rc-no-auth"]
# At shutdown, how long to wait for rclone to finish uploading. The Host
# Agent gives the whole guest 90 s before it kills the VM.
FLUSH_WAIT_SECONDS = 75
FLUSH_POLL_SECONDS = 1


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


def _json_post(
    url: str, token: str, payload: dict | None = None, timeout: float = 30
) -> dict:
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
    with urllib.request.urlopen(request, timeout=timeout) as response:
        raw = response.read()
    if not raw:
        return {}
    body = json.loads(raw.decode("utf-8"))
    if not isinstance(body, dict):
        raise RuntimeError("guest bootstrap response was not an object")
    return body


def redeem_bootstrap(origin: str, token: str, sleep=time.sleep, post=None) -> dict:
    """Fetch this session's secrets, retrying while the network comes up.

    A refusal from the Matchmaker (HTTP 4xx: the token is wrong or the
    session is over) is final and is not retried.
    """
    post = post or _json_post
    last_error: Exception | None = None
    for attempt in range(len(BOOTSTRAP_RETRY_DELAYS) + 1):
        try:
            return post(
                f"{origin}/v1/guest/bootstrap", token, timeout=BOOTSTRAP_TIMEOUT_SECONDS
            )
        except urllib.error.HTTPError as error:
            if 400 <= error.code < 500:
                raise
            last_error = error
        except (urllib.error.URLError, OSError) as error:
            last_error = error
        if attempt < len(BOOTSTRAP_RETRY_DELAYS):
            sleep(BOOTSTRAP_RETRY_DELAYS[attempt])
    assert last_error is not None
    raise last_error


def mark_session_vm() -> None:
    """Tell the desktop this VM serves a session: never run without a password."""
    RUN_DIR.mkdir(parents=True, exist_ok=True)
    SESSION_MARK_PATH.touch()


def store_desktop_password(body: dict) -> bool:
    """Write the session's desktop password where only root can read it."""
    password = body.get("desktopPassword")
    if not isinstance(password, str) or not password.strip():
        return False
    password = password.strip()
    if "\n" in password or "\r" in password:
        raise RuntimeError("desktop password must be a single line")
    RUN_DIR.mkdir(parents=True, exist_ok=True)
    # Created 0600 before anything is written to it.
    fd = os.open(DESKTOP_PASSWORD_PATH, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        handle.write(password + "\n")
    os.chmod(DESKTOP_PASSWORD_PATH, 0o600)
    return True


def _valid_stream_token(value: object) -> str | None:
    if not isinstance(value, str):
        return None
    v = value.strip().lower()
    if len(v) == 64 and all(c in "0123456789abcdef" for c in v):
        return v
    return None


def read_manual_stream_token() -> str | None:
    """dcl.stream_token from the kernel command line or SMBIOS, for a LAN
    test without a Matchmaker (see docs/runbooks/stream-lan-test.md)."""
    cmdline = _read_text(Path(CMDLINE_FILE)) or ""
    value = _cmdline_value(cmdline, "dcl.stream_token")
    if not value:
        value = _oem_value(_smbios_oem_strings(), "dcl.stream_token")
    return _valid_stream_token(value)


def store_stream_token(value: object, user: str | None = None) -> bool:
    """Write the session's stream token where only the desktop user can read it."""
    token = _valid_stream_token(value)
    if token is None:
        if value not in (None, ""):
            raise RuntimeError("stream token must be 64 hex characters")
        return False
    RUN_DIR.mkdir(parents=True, exist_ok=True)
    fd = os.open(STREAM_TOKEN_PATH, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        handle.write(token + "\n")
    os.chmod(STREAM_TOKEN_PATH, 0o600)
    try:
        entry = pwd.getpwnam(user or DESKTOP_USER)
        os.chown(STREAM_TOKEN_PATH, entry.pw_uid, entry.pw_gid)
    except (KeyError, PermissionError):
        pass
    return True


def mount_owner_args(user: str | None = None) -> list[str]:
    """Flags that give the mount to the desktop user.

    This service runs as root. Without these the files under ~/Persistent
    would belong to root and the desktop user could not open or save them.
    """
    try:
        entry = pwd.getpwnam(user or DESKTOP_USER)
    except KeyError:
        return []
    return [
        "--allow-other",
        "--uid",
        str(entry.pw_uid),
        "--gid",
        str(entry.pw_gid),
        "--umask",
        "077",
    ]


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
    # Short-lived per-user credentials (Cloudflare R2) come with a session token.
    token = storage.get("sessionToken")
    if isinstance(token, str) and token:
        env["AWS_SESSION_TOKEN"] = token
    else:
        env.pop("AWS_SESSION_TOKEN", None)
    return env


def rclone_argv(storage: dict) -> list[str]:
    args = storage.get("rcloneArgs")
    if not isinstance(args, list) or not args:
        raise RuntimeError("bootstrap storage missing rcloneArgs")
    argv = ["rclone"] + [str(item) for item in args]
    if "--s3-env-auth" not in argv:
        raise RuntimeError("guest rclone must use --s3-env-auth")
    if "--rc" not in argv:
        argv += RC_ARGS
    if "--uid" not in argv:
        argv += mount_owner_args()
    return argv


def pending_uploads(stats: dict) -> int:
    """Files rclone still has to upload, from `rclone rc vfs/stats`."""
    cache = stats.get("diskCache")
    if not isinstance(cache, dict):
        return 0
    total = 0
    for key in ("uploadsInProgress", "uploadsQueued"):
        value = cache.get(key)
        if isinstance(value, int):
            total += value
    return total


def _vfs_stats() -> dict | None:
    try:
        out = subprocess.run(
            ["rclone", "rc", "--url", f"http://{RC_ADDR}/", "vfs/stats"],
            capture_output=True,
            text=True,
            timeout=10,
            check=True,
        ).stdout
        body = json.loads(out)
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError):
        return None
    return body if isinstance(body, dict) else None


def wait_for_uploads(stats=_vfs_stats, sleep=time.sleep, clock=time.monotonic) -> bool:
    """Wait until rclone has uploaded every closed file. False on timeout.

    rclone uploads a file a few seconds after it is closed; the session disk
    is thrown away when the VM stops, so anything still queued would be lost.
    No answer from rclone means there is no mount to wait for.
    """
    deadline = clock() + FLUSH_WAIT_SECONDS
    while True:
        body = stats()
        if body is None or pending_uploads(body) == 0:
            return True
        if clock() >= deadline:
            return False
        sleep(FLUSH_POLL_SECONDS)


def flush_storage() -> bool:
    """Upload what is pending, then unmount so nothing new is written."""
    if DRY_RUN:
        return True
    done = wait_for_uploads()
    if not done:
        print("dcl-guest-join: uploads still pending at shutdown", file=sys.stderr)
    subprocess.run(["fusermount", "-u", MOUNT_POINT], capture_output=True, check=False)
    return done


def _run(argv: list[str], env: dict[str, str] | None = None) -> None:
    if DRY_RUN:
        print(" ".join(argv))
        return
    subprocess.run(argv, check=True, env=env)


def main(argv: list[str]) -> int:
    flush_only = "--flush" in argv
    if not flush_only and not DRY_RUN:
        manual = read_manual_stream_token()
        if manual:
            store_stream_token(manual)
    origin, token = read_bootstrap()
    if not origin or not token:
        # Image can boot without a session (packer, Hyper-V leftover).
        return 0
    if flush_only:
        flushed = flush_storage()
        if not flushed:
            return 1
        try:
            report_saved_as_of(origin, token)
        except (urllib.error.URLError, RuntimeError, json.JSONDecodeError):
            return 1
        return 0

    try:
        if not DRY_RUN:
            mark_session_vm()
        body = redeem_bootstrap(origin, token)
        # Before the mesh join: the desktop must never be reachable without it.
        if not DRY_RUN:
            store_desktop_password(body)
            # Pro: the streamer's session token (Matchmaker side: phase 5).
            store_stream_token(body.get("streamToken"))
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
    except (
        urllib.error.URLError,
        OSError,
        RuntimeError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
    ) as error:
        # Shown on the serial console and in the journal. Holds no secret:
        # a failed command is named by its program only, never its arguments.
        if isinstance(error, subprocess.CalledProcessError):
            program = error.cmd[0] if isinstance(error.cmd, list) and error.cmd else "command"
            detail = f"{program} exited with status {error.returncode}"
        else:
            detail = f"{type(error).__name__}: {error}"
        print(f"dcl-guest-join: failed: {detail}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
