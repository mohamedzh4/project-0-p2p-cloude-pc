"""Environment-backed settings. Secrets never have in-code fallbacks."""

from __future__ import annotations

from dataclasses import dataclass, field
from functools import lru_cache
from pathlib import Path
import base64
import os
import re

from dotenv import load_dotenv

from matchmaker.headscale import DEFAULT_EXIT_NODE


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


def _headscale_exit_node() -> str | None:
    """Platform exit-node hostname/IP for guest `--exit-node`.

    Defaults to the compose overlay hostname `dcl-platform-exit`. Set
    `HEADSCALE_EXIT_NODE=off` to omit `--exit-node` from guest join plans
    (then architecture §9 item 5 is not auto-applied).
    """
    raw = os.environ.get("HEADSCALE_EXIT_NODE")
    if raw is None or raw.strip() == "":
        return DEFAULT_EXIT_NODE
    value = raw.strip()
    if value.lower() in {"off", "none", "false"}:
        return None
    return value


def _matchmaker_public_origin() -> str | None:
    """Origin guests call for POST /v1/guest/bootstrap.

    Unset omits bootstrapOrigin from the provision hint (honest: the guest
    image cannot join). http is allowed — lab compose and TAP guests may not
    have TLS. Do not invent a domain.
    """
    return _optional_http_url("MATCHMAKER_PUBLIC_ORIGIN")


def _gateway_public_origin() -> str | None:
    """Public HTTPS origin the web app opens (zero-install Standard Lab).

    Unset keeps session JSON on the mesh locator (local lab). Production
    browser-only access requires this plus GATEWAY_RESOLVE_TOKEN. Do not
    invent a domain — the operator supplies one they control DNS for.
    """
    raw = os.environ.get("GATEWAY_PUBLIC_ORIGIN")
    if raw is None or raw.strip() == "":
        return None
    value = raw.strip().rstrip("/")
    if not value.startswith("https://"):
        raise RuntimeError("GATEWAY_PUBLIC_ORIGIN must be an https URL")
    return value


def _gateway_resolve_token(origin: str | None) -> str | None:
    raw = os.environ.get("GATEWAY_RESOLVE_TOKEN")
    if raw is None or raw.strip() == "":
        if origin is not None:
            raise RuntimeError(
                "GATEWAY_RESOLVE_TOKEN is required when GATEWAY_PUBLIC_ORIGIN is set"
            )
        return None
    value = raw.strip()
    if len(value) < 16:
        raise RuntimeError("GATEWAY_RESOLVE_TOKEN must be at least 16 characters")
    return value


def _gateway_token_ttl_seconds() -> int:
    raw = os.environ.get("GATEWAY_TOKEN_TTL_SECONDS", "43200").strip()
    if raw == "":
        raw = "43200"
    try:
        ttl = int(raw)
    except ValueError as exc:
        raise RuntimeError("GATEWAY_TOKEN_TTL_SECONDS must be an integer") from exc
    if ttl <= 0:
        raise RuntimeError("GATEWAY_TOKEN_TTL_SECONDS must be positive")
    return ttl


def _guest_s3_endpoint() -> str:
    """URL guests rclone against. Defaults to the Matchmaker→renterd endpoint."""
    guest_name = _first_nonempty_env("S3_GUEST_ENDPOINT", "RENTERD_S3_GUEST_ENDPOINT")
    if guest_name is not None:
        guest = _optional_http_url(guest_name)
        if guest is not None:
            return guest
    return _require_first_http_url("S3_ENDPOINT", "RENTERD_S3_ENDPOINT")


def _positive_int(name: str, default: int) -> int:
    raw = os.environ.get(name, "").strip() or str(default)
    try:
        value = int(raw)
    except ValueError as exc:
        raise RuntimeError(f"{name} must be an integer") from exc
    if value <= 0:
        raise RuntimeError(f"{name} must be positive")
    return value


def _optional_env(name: str) -> str | None:
    raw = os.environ.get(name)
    if raw is None or raw.strip() == "":
        return None
    return raw.strip()


def _storage_credentials_mode() -> str:
    """`r2` issues per-user temporary keys. `static` hands every guest the
    Matchmaker's own key and is for local development only."""
    mode = (os.environ.get("STORAGE_CREDENTIALS", "").strip() or "static").lower()
    if mode not in {"static", "r2"}:
        raise RuntimeError("STORAGE_CREDENTIALS must be 'static' or 'r2'")
    return mode


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
    headscale_exit_node: str | None
    renterd_s3_endpoint: str
    renterd_s3_guest_endpoint: str
    renterd_s3_region: str
    renterd_s3_access_key: str = field(repr=False)
    renterd_s3_secret_key: str = field(repr=False)
    gateway_public_origin: str | None
    gateway_resolve_token: str | None = field(repr=False)
    gateway_token_ttl_seconds: int
    matchmaker_public_origin: str | None
    storage_credentials_mode: str = "static"
    storage_shared_bucket: str | None = None
    storage_quota_bytes: int = 20 * 1024**3
    storage_credential_ttl_seconds: int = 43200
    r2_account_id: str | None = None
    r2_api_token: str | None = field(default=None, repr=False)
    # Cloudflare API origin. Only the integration test overrides it.
    r2_api_base: str | None = None
    provision_timeout_seconds: int = 420
    # Setup kits downloaded from the website (see host_kits.py). Kits are
    # offered only when the public key, the disk and the installer are set.
    image_signing_public_key: str | None = None
    host_kit_qcow2_url: str | None = None
    host_kit_windows_installer_url: str | None = None
    host_kit_deb_url: str | None = None
    host_kit_appimage_url: str | None = None
    host_kit_scripts_dir: str | None = None

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

        gateway_origin = _gateway_public_origin()
        storage_mode = _storage_credentials_mode()
        r2_account_id = _optional_env("R2_ACCOUNT_ID")
        r2_api_token = _optional_env("R2_API_TOKEN")
        shared_bucket = _optional_env("S3_BUCKET")
        if storage_mode == "r2":
            # R2_API_TOKEN is optional: without it the per-user keys are
            # signed on this server with the parent S3 key.
            if r2_account_id is None:
                raise RuntimeError("R2_ACCOUNT_ID is required when STORAGE_CREDENTIALS=r2")
            if shared_bucket is None:
                raise RuntimeError("S3_BUCKET is required when STORAGE_CREDENTIALS=r2")
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
            headscale_exit_node=_headscale_exit_node(),
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
            gateway_public_origin=gateway_origin,
            gateway_resolve_token=_gateway_resolve_token(gateway_origin),
            gateway_token_ttl_seconds=_gateway_token_ttl_seconds(),
            matchmaker_public_origin=_matchmaker_public_origin(),
            storage_credentials_mode=storage_mode,
            storage_shared_bucket=shared_bucket,
            storage_quota_bytes=_positive_int("STORAGE_QUOTA_GB", 20) * 1024**3,
            storage_credential_ttl_seconds=_positive_int(
                "STORAGE_CREDENTIAL_TTL_SECONDS", 43200
            ),
            r2_account_id=r2_account_id,
            r2_api_token=r2_api_token,
            r2_api_base=_optional_http_url("R2_API_BASE"),
            provision_timeout_seconds=_positive_int("PROVISION_TIMEOUT_SECONDS", 420),
            image_signing_public_key=_optional_env("IMAGE_SIGNING_PUBLIC_KEY"),
            host_kit_qcow2_url=_optional_http_url("HOST_KIT_QCOW2_URL"),
            host_kit_windows_installer_url=_optional_http_url("HOST_KIT_WINDOWS_INSTALLER_URL"),
            host_kit_deb_url=_optional_http_url("HOST_KIT_DEB_URL"),
            host_kit_appimage_url=_optional_http_url("HOST_KIT_APPIMAGE_URL"),
            host_kit_scripts_dir=_optional_env("HOST_KIT_SCRIPTS_DIR"),
        )


@lru_cache
def get_settings() -> Settings:
    load_env_files()
    return Settings.from_env()
