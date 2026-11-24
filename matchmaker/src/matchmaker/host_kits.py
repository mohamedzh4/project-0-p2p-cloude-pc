"""Host setup kits downloaded from the website.

A signed-in user asks for a kit for Windows or Linux and gets a small zip:
the setup script plus a `friend.env`. That file holds a registration code
made for this download (never the shared HOST_BOOTSTRAP_TOKEN), the public
URLs and the image pins. The setup script downloads the Host Agent installer
and the guest disk itself; the agent checks the disk's signature.

The code works like the bootstrap Bearer, with limits:
- unused, it registers one new host before it expires;
- after that it belongs to that host only (with the host's own secret);
- the host starts as pending and still needs the operator's approval.
"""

from __future__ import annotations

import hashlib
import io
import secrets
import zipfile
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from uuid import UUID

from sqlalchemy import delete, select
from sqlalchemy.orm import Session

from matchmaker.config import Settings
from matchmaker.models import HostEnrollmentCode

CODE_TTL = timedelta(days=7)
# Unused codes a user may hold at once; older ones are dropped.
MAX_UNUSED_CODES = 3
CODE_PREFIX = "dclk_"

OS_WINDOWS = "windows"
OS_LINUX = "linux"
KIT_OSES = (OS_WINDOWS, OS_LINUX)

WINDOWS_FILES = ("setup.cmd", "windows-host-setup.ps1")
LINUX_FILES = ("linux-host-setup.sh", "linux-host-uninstall.sh")

README = {
    OS_WINDOWS: (
        "Share this PC\r\n"
        "=============\r\n"
        "1. Unzip this folder.\r\n"
        "2. Double-click setup.cmd and click Yes when Windows asks.\r\n"
        "   If Windows says \"Windows protected your PC\", click\r\n"
        "   \"More info\", then \"Run anyway\" (the installer is not\r\n"
        "   code-signed yet).\r\n"
        "3. Setup downloads Host Agent and the desktop disk (about 1.3 GB).\r\n"
        "4. Open Host Agent, choose what to share, click \"Save and register\".\r\n"
        "Your PC is offered to renters once the operator approves it.\r\n"
        "friend.env holds this PC's registration code: do not share it.\r\n"
    ),
    OS_LINUX: (
        "Share this PC\n"
        "=============\n"
        "Ubuntu/Debian or Arch. In this folder run:\n"
        "    bash linux-host-setup.sh\n"
        "Setup downloads Host Agent and the desktop disk (about 1.3 GB).\n"
        "Then open Host Agent, choose what to share, click \"Save and register\".\n"
        "Your PC is offered to renters once the operator approves it.\n"
        "friend.env holds this PC's registration code: do not share it.\n"
        "To remove everything later: bash linux-host-uninstall.sh\n"
    ),
}


class KitUnavailable(Exception):
    """Kits are not configured on this server (message names what is missing)."""


def hash_code(code: str) -> str:
    return hashlib.sha256(code.encode("utf-8")).hexdigest()


def looks_like_code(value: str) -> bool:
    return value.startswith(CODE_PREFIX) and 30 <= len(value) <= 80


def mint_code(
    db: Session, *, user_id: UUID, lab_os: str, now: datetime | None = None
) -> str:
    moment = now or datetime.now(timezone.utc)
    unused = db.scalars(
        select(HostEnrollmentCode)
        .where(HostEnrollmentCode.user_id == user_id, HostEnrollmentCode.host_id.is_(None))
        .order_by(HostEnrollmentCode.created_at.desc())
    ).all()
    stale = [row.id for row in unused[MAX_UNUSED_CODES - 1 :]]
    stale += [row.id for row in unused if row.expires_at <= moment]
    if stale:
        db.execute(delete(HostEnrollmentCode).where(HostEnrollmentCode.id.in_(stale)))
    code = CODE_PREFIX + secrets.token_urlsafe(32)
    db.add(
        HostEnrollmentCode(
            user_id=user_id,
            code_hash=hash_code(code),
            lab_os=lab_os,
            expires_at=moment + CODE_TTL,
        )
    )
    db.commit()
    return code


def find_code(db: Session, code: str, *, now: datetime | None = None) -> HostEnrollmentCode | None:
    """A code that may still be used: bound to a host, or unused and unexpired."""
    if not looks_like_code(code):
        return None
    row = db.scalars(
        select(HostEnrollmentCode).where(HostEnrollmentCode.code_hash == hash_code(code))
    ).first()
    if row is None:
        return None
    if row.host_id is None and row.expires_at <= (now or datetime.now(timezone.utc)):
        return None
    return row


def _scripts_dir(settings: Settings) -> Path:
    if settings.host_kit_scripts_dir:
        return Path(settings.host_kit_scripts_dir)
    # matchmaker/src/matchmaker/host_kits.py -> repo/host-agent/scripts
    return Path(__file__).resolve().parents[3] / "host-agent" / "scripts"


@dataclass(frozen=True)
class Kit:
    filename: str
    body: bytes


def kit_env(settings: Settings, *, lab_os: str, code: str) -> str:
    """friend.env for a kit. Raises KitUnavailable when a value is missing."""
    missing = []
    origin = settings.matchmaker_public_origin
    if not origin:
        missing.append("MATCHMAKER_PUBLIC_ORIGIN")
    if not settings.image_signing_public_key:
        missing.append("IMAGE_SIGNING_PUBLIC_KEY")
    if not settings.host_kit_qcow2_url:
        missing.append("HOST_KIT_QCOW2_URL")
    if lab_os == OS_WINDOWS and not settings.host_kit_windows_installer_url:
        missing.append("HOST_KIT_WINDOWS_INSTALLER_URL")
    if lab_os == OS_LINUX and not (settings.host_kit_deb_url or settings.host_kit_appimage_url):
        missing.append("HOST_KIT_DEB_URL or HOST_KIT_APPIMAGE_URL")
    if missing:
        raise KitUnavailable(", ".join(missing))
    lines = [
        f"HOST_BOOTSTRAP_TOKEN={code}",
        f"MATCHMAKER_URL={origin}",
        f"HEADSCALE_LOGIN_SERVER={settings.headscale_login_server}",
        f"IMAGE_SIGNING_PUBLIC_KEY={settings.image_signing_public_key}",
        f"IMAGE_STANDARD_DIGEST={settings.image_standard_digest}",
        f"IMAGE_STANDARD_SIGNATURE={settings.image_standard_signature}",
        "HOST_DISPLAY_NAME=",
        f"IMAGE_QCOW2_URL={settings.host_kit_qcow2_url}",
    ]
    if lab_os == OS_WINDOWS:
        lines.append(f"HOST_AGENT_INSTALLER_URL={settings.host_kit_windows_installer_url}")
    else:
        if settings.host_kit_deb_url:
            lines.append(f"HOST_AGENT_DEB_URL={settings.host_kit_deb_url}")
        if settings.host_kit_appimage_url:
            lines.append(f"HOST_AGENT_APPIMAGE_URL={settings.host_kit_appimage_url}")
    newline = "\r\n" if lab_os == OS_WINDOWS else "\n"
    return newline.join(lines) + newline


def build_kit(settings: Settings, *, lab_os: str, code: str) -> Kit:
    if lab_os not in KIT_OSES:
        raise ValueError(lab_os)
    env = kit_env(settings, lab_os=lab_os, code=code)
    scripts = _scripts_dir(settings)
    names = WINDOWS_FILES if lab_os == OS_WINDOWS else LINUX_FILES
    folder = "dcl-host-setup"
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name in names:
            path = scripts / name
            if not path.is_file():
                raise KitUnavailable(f"setup file {name} is missing on the server")
            info = zipfile.ZipInfo(f"{folder}/{name}")
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (0o755 if name.endswith(".sh") else 0o644) << 16
            archive.writestr(info, path.read_bytes())
        secret = zipfile.ZipInfo(f"{folder}/friend.env")
        secret.compress_type = zipfile.ZIP_DEFLATED
        secret.external_attr = 0o600 << 16
        archive.writestr(secret, env)
        archive.writestr(f"{folder}/README.txt", README[lab_os])
    return Kit(filename=f"dcl-host-setup-{lab_os}.zip", body=buffer.getvalue())
