"""Environment-backed settings. Secrets never have in-code fallbacks."""

from __future__ import annotations

from dataclasses import dataclass, field
from functools import lru_cache
from pathlib import Path
import base64
import os
import re

from dotenv import load_dotenv


def load_env_files() -> None:
    """Load `.env` from the process cwd and from the matchmaker/ directory."""
    load_dotenv()
    matchmaker_root = Path(__file__).resolve().parents[2]
    load_dotenv(matchmaker_root / ".env")


def _require_env(name: str) -> str:
    value = os.environ.get(name)
    if value is None or value.strip() == "":
        raise RuntimeError(f"{name} is required")
    return value


def _require_http_url(name: str) -> str:
    value = _optional_http_url(name)
    if value is None:
        raise RuntimeError(f"{name} is required")
    return value


def _optional_http_url(name: str) -> str | None:
    raw = os.environ.get(name)
    if raw is None or raw.strip() == "":
        return None
    value = raw.strip().rstrip("/")
    if not (value.startswith("http://") or value.startswith("https://")):
        raise RuntimeError(f"{name} must be an http or https URL")
    return value


_SHA256_HEX = re.compile(r"^[0-9a-fA-F]{64}$")


def _require_sha256_hex(name: str) -> str:
    raw = _require_env(name).strip()
    if raw.lower().startswith("sha256:"):
        raw = raw[7:]
    if _SHA256_HEX.fullmatch(raw) is None:
        raise RuntimeError(f"{name} must be a 64-character SHA-256 hex digest")
    return raw.lower()


def _first_nonempty_env(*names: str) -> str | None:
    for name in names:
        raw = os.environ.get(name)
        if raw is not None and raw.strip() != "":
            return name
    return None


def _require_first_http_url(*names: str) -> str:
    chosen = _first_nonempty_env(*names)
    if chosen is None:
        raise RuntimeError(f"{names[0]} is required")
    return _require_http_url(chosen)


def _require_first_s3_key(*names: str) -> str:
    chosen = _first_nonempty_env(*names)
    if chosen is None:
        raise RuntimeError(f"{names[0]} is required")
    return _require_s3_key(chosen)


def _s3_region() -> str:
    chosen = _first_nonempty_env("S3_REGION", "RENTERD_S3_REGION")
    if chosen is None:
        return "us-east-1"
    return os.environ[chosen].strip()


def _guest_s3_endpoint() -> str:
    """URL guests rclone against. Defaults to the Matchmaker→renterd endpoint."""
    guest_name = _first_nonempty_env("S3_GUEST_ENDPOINT", "RENTERD_S3_GUEST_ENDPOINT")
    if guest_name is not None:
        guest = _optional_http_url(guest_name)
        if guest is not None:
            return guest
    return _require_first_http_url("S3_ENDPOINT", "RENTERD_S3_ENDPOINT")


def _require_s3_key(name: str) -> str:
    value = _require_env(name)
    if len(value) < 16:
        raise RuntimeError(f"{name} must be at least 16 characters")
    return value


def _require_ed25519_signature(name: str) -> str:
    raw = _require_env(name).strip()
    try:
        decoded = base64.b64decode(raw, validate=True)
    except (ValueError, TypeError) as exc:
        raise RuntimeError(
            f"{name} must be standard base64 of a 64-byte Ed25519 signature"
        ) from exc
    if len(decoded) != 64:
        raise RuntimeError(
            f"{name} must be standard base64 of a 64-byte Ed25519 signature"
        )
    return base64.b64encode(decoded).decode("ascii")


@dataclass(frozen=True)
class Settings:
    database_url: str
    jwt_secret: str = field(repr=False)
    jwt_expire_minutes: int
    host_bootstrap_token: str = field(repr=False)
    host_approve_token: str = field(repr=False)
    host_agent_provision_url: str | None
    credits_per_minute: int
    stale_session_sweep_interval_seconds: int
    image_standard_digest: str
    image_standard_signature: str
    image_pro_digest: str
    image_pro_signature: str
    headscale_url: str
    headscale_api_key: str = field(repr=False)
    headscale_user: str
    headscale_login_server: str
    renterd_s3_endpoint: str
    renterd_s3_guest_endpoint: str
    renterd_s3_region: str
    renterd_s3_access_key: str = field(repr=False)
    renterd_s3_secret_key: str = field(repr=False)

    @classmethod
    def from_env(cls) -> Settings:
        expire_raw = os.environ.get("JWT_EXPIRE_MINUTES", "1440").strip()
        try:
            expire_minutes = int(expire_raw)
        except ValueError as exc:
            raise RuntimeError("JWT_EXPIRE_MINUTES must be an integer") from exc
        if expire_minutes <= 0:
            raise RuntimeError("JWT_EXPIRE_MINUTES must be positive")

        credits_raw = _require_env("CREDITS_PER_MINUTE")
        try:
            credits_per_minute = int(credits_raw)
        except ValueError as exc:
            raise RuntimeError("CREDITS_PER_MINUTE must be an integer") from exc
        if credits_per_minute < 0:
            raise RuntimeError("CREDITS_PER_MINUTE must be non-negative")

        host_bootstrap_token = _require_env("HOST_BOOTSTRAP_TOKEN")
        host_approve_token = _require_env("HOST_APPROVE_TOKEN")
        if host_bootstrap_token == host_approve_token:
            raise RuntimeError("HOST_BOOTSTRAP_TOKEN and HOST_APPROVE_TOKEN must differ")

        sweep_raw = os.environ.get("STALE_SESSION_SWEEP_INTERVAL_SECONDS", "15").strip()
        if sweep_raw == "":
            sweep_raw = "15"
        try:
            sweep_interval = int(sweep_raw)
        except ValueError as exc:
            raise RuntimeError(
                "STALE_SESSION_SWEEP_INTERVAL_SECONDS must be an integer"
            ) from exc
        if sweep_interval < 0:
            raise RuntimeError("STALE_SESSION_SWEEP_INTERVAL_SECONDS must be non-negative")

        return cls(
            database_url=_require_env("DATABASE_URL"),
            jwt_secret=_require_env("JWT_SECRET"),
            jwt_expire_minutes=expire_minutes,
            host_bootstrap_token=host_bootstrap_token,
            host_approve_token=host_approve_token,
            host_agent_provision_url=_optional_http_url("HOST_AGENT_PROVISION_URL"),
            credits_per_minute=credits_per_minute,
            stale_session_sweep_interval_seconds=sweep_interval,
            image_standard_digest=_require_sha256_hex("IMAGE_STANDARD_DIGEST"),
            image_standard_signature=_require_ed25519_signature("IMAGE_STANDARD_SIGNATURE"),
            image_pro_digest=_require_sha256_hex("IMAGE_PRO_DIGEST"),
            image_pro_signature=_require_ed25519_signature("IMAGE_PRO_SIGNATURE"),
            headscale_url=_require_http_url("HEADSCALE_URL"),
            headscale_api_key=_require_env("HEADSCALE_API_KEY"),
            headscale_user=_require_env("HEADSCALE_USER"),
            headscale_login_server=_require_http_url("HEADSCALE_LOGIN_SERVER"),
            renterd_s3_endpoint=_require_first_http_url(
                "S3_ENDPOINT", "RENTERD_S3_ENDPOINT"
            ),
            renterd_s3_guest_endpoint=_guest_s3_endpoint(),
            renterd_s3_region=_s3_region(),
            renterd_s3_access_key=_require_first_s3_key(
                "S3_ACCESS_KEY", "RENTERD_S3_ACCESS_KEY"
            ),
            renterd_s3_secret_key=_require_first_s3_key(
                "S3_SECRET_KEY", "RENTERD_S3_SECRET_KEY"
            ),
        )


@lru_cache
def get_settings() -> Settings:
    load_env_files()
    return Settings.from_env()
