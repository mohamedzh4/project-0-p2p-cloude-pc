"""Environment-backed settings. Secrets never have in-code fallbacks."""

from __future__ import annotations

from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path

from dotenv import load_dotenv
import os


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


@dataclass(frozen=True)
class Settings:
    database_url: str
    jwt_secret: str
    jwt_expire_minutes: int
    host_bootstrap_token: str
    host_approve_token: str

    @classmethod
    def from_env(cls) -> Settings:
        expire_raw = os.environ.get("JWT_EXPIRE_MINUTES", "1440").strip()
        try:
            expire_minutes = int(expire_raw)
        except ValueError as exc:
            raise RuntimeError("JWT_EXPIRE_MINUTES must be an integer") from exc
        if expire_minutes <= 0:
            raise RuntimeError("JWT_EXPIRE_MINUTES must be positive")

        host_bootstrap_token = _require_env("HOST_BOOTSTRAP_TOKEN")
        host_approve_token = _require_env("HOST_APPROVE_TOKEN")
        if host_bootstrap_token == host_approve_token:
            raise RuntimeError("HOST_BOOTSTRAP_TOKEN and HOST_APPROVE_TOKEN must differ")

        return cls(
            database_url=_require_env("DATABASE_URL"),
            jwt_secret=_require_env("JWT_SECRET"),
            jwt_expire_minutes=expire_minutes,
            host_bootstrap_token=host_bootstrap_token,
            host_approve_token=host_approve_token,
        )


@lru_cache
def get_settings() -> Settings:
    load_env_files()
    return Settings.from_env()
