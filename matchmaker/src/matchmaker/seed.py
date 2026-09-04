"""Insert or update the single Phase 1 approved host from environment variables."""

from __future__ import annotations

import os
import sys

from sqlalchemy import select

from matchmaker.config import get_settings, load_env_files
from matchmaker.db import get_session_factory
from matchmaker.models import Host, HostStatus, LabTier


def _require_env(name: str) -> str:
    value = os.environ.get(name)
    if value is None or value.strip() == "":
        raise RuntimeError(f"{name} is required")
    return value.strip()


def _optional_env(name: str) -> str | None:
    value = os.environ.get(name)
    if value is None or value.strip() == "":
        return None
    return value.strip()


def _int_env(name: str, default: int) -> int:
    raw = os.environ.get(name)
    if raw is None or raw.strip() == "":
        return default
    return int(raw)


def _bool_env(name: str, default: bool) -> bool:
    raw = os.environ.get(name)
    if raw is None or raw.strip() == "":
        return default
    return raw.strip().lower() in {"1", "true", "yes", "on"}


def seed_host() -> Host:
    load_env_files()
    get_settings()

    display_name = _optional_env("SEED_HOST_DISPLAY_NAME") or "phase1-lab-1"
    kasm_url = _require_env("SEED_HOST_KASM_URL")
    kasm_token = _optional_env("SEED_HOST_KASM_TOKEN")
    shared_logical_cores = _int_env("SEED_HOST_SHARED_LOGICAL_CORES", 4)
    shared_memory_bytes = _int_env("SEED_HOST_SHARED_MEMORY_BYTES", 8 * 1024 * 1024 * 1024)
    share_gpu = _bool_env("SEED_HOST_SHARE_GPU", False)
    shared_gpu_id = _optional_env("SEED_HOST_SHARED_GPU_ID")

    db = get_session_factory()()
    try:
        existing = db.scalars(select(Host).limit(1)).first()
        if existing is None:
            host = Host(
                display_name=display_name,
                lab_tier=LabTier.STANDARD,
                shared_logical_cores=shared_logical_cores,
                shared_memory_bytes=shared_memory_bytes,
                share_gpu=share_gpu,
                shared_gpu_id=shared_gpu_id,
                kasm_url=kasm_url,
                kasm_token=kasm_token,
                status=HostStatus.APPROVED,
            )
            db.add(host)
            db.commit()
            db.refresh(host)
            return host

        existing.display_name = display_name
        existing.kasm_url = kasm_url
        existing.kasm_token = kasm_token
        existing.shared_logical_cores = shared_logical_cores
        existing.shared_memory_bytes = shared_memory_bytes
        existing.share_gpu = share_gpu
        existing.shared_gpu_id = shared_gpu_id
        existing.lab_tier = LabTier.STANDARD
        existing.status = HostStatus.APPROVED
        db.commit()
        db.refresh(existing)
        return existing
    finally:
        db.close()


def main() -> None:
    try:
        host = seed_host()
    except Exception as exc:
        print(f"seed failed: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
    token_note = "present" if host.kasm_token else "absent"
    print(
        f"seeded host {host.id} ({host.display_name}) "
        f"kasmUrl={host.kasm_url} kasmToken={token_note}"
    )


if __name__ == "__main__":
    main()
