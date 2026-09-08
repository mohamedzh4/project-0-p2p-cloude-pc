"""Environment-backed gateway settings. Secrets never have in-code fallbacks."""

from __future__ import annotations

from dataclasses import dataclass, field
from functools import lru_cache
from pathlib import Path
import os

from dotenv import load_dotenv


def load_env_files() -> None:
    load_dotenv()
    gateway_root = Path(__file__).resolve().parents[2]
    load_dotenv(gateway_root / ".env")


def _require_env(name: str) -> str:
    value = os.environ.get(name)
    if value is None or value.strip() == "":
        raise RuntimeError(f"{name} is required")
    return value


def _require_http_url(name: str) -> str:
    raw = _require_env(name).strip().rstrip("/")
    if not (raw.startswith("http://") or raw.startswith("https://")):
        raise RuntimeError(f"{name} must be an http or https URL")
    return raw


def _positive_int(name: str, default: str) -> int:
    raw = os.environ.get(name, default).strip()
    if raw == "":
        raw = default
    try:
        value = int(raw)
    except ValueError as exc:
        raise RuntimeError(f"{name} must be an integer") from exc
    if value <= 0:
        raise RuntimeError(f"{name} must be positive")
    return value


def _listen_host() -> str:
    raw = os.environ.get("GATEWAY_LISTEN_HOST", "127.0.0.1").strip()
    return raw or "127.0.0.1"


@dataclass(frozen=True)
class Settings:
    matchmaker_url: str
    resolve_token: str = field(repr=False)
    fail_limit: int
    fail_window_seconds: int
    allow_loopback_target: bool
    listen_host: str
    listen_port: int
    public_origin: str | None

    @classmethod
    def from_env(cls) -> Settings:
        token = _require_env("GATEWAY_RESOLVE_TOKEN")
        if len(token) < 16:
            raise RuntimeError("GATEWAY_RESOLVE_TOKEN must be at least 16 characters")
        origin_raw = os.environ.get("GATEWAY_PUBLIC_ORIGIN", "").strip().rstrip("/")
        origin = origin_raw or None
        if origin is not None and not origin.startswith("https://"):
            raise RuntimeError("GATEWAY_PUBLIC_ORIGIN must be an https URL")
        loopback = os.environ.get("GATEWAY_ALLOW_LOOPBACK_TARGET", "").strip().lower()
        return cls(
            matchmaker_url=_require_http_url("MATCHMAKER_URL"),
            resolve_token=token,
            fail_limit=_positive_int("GATEWAY_FAIL_LIMIT", "20"),
            fail_window_seconds=_positive_int("GATEWAY_FAIL_WINDOW_SECONDS", "60"),
            allow_loopback_target=loopback in {"1", "true", "yes"},
            listen_host=_listen_host(),
            listen_port=_positive_int("GATEWAY_LISTEN_PORT", "7843"),
            public_origin=origin,
        )


@lru_cache
def get_settings() -> Settings:
    load_env_files()
    return Settings.from_env()
