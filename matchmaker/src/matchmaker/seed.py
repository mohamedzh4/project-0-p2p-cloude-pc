"""Insert or update the approved seed host, or manually approve a pending host."""

from __future__ import annotations

import argparse
import os
import sys
from datetime import datetime, timezone
from uuid import UUID

from sqlalchemy import select

from matchmaker.config import get_settings, load_env_files
from matchmaker.db import get_session_factory
from matchmaker.inventory import approve_host
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
    agent_url = _optional_env("SEED_HOST_AGENT_URL")
    shared_logical_cores = _int_env("SEED_HOST_SHARED_LOGICAL_CORES", 4)
    shared_memory_bytes = _int_env("SEED_HOST_SHARED_MEMORY_BYTES", 8 * 1024 * 1024 * 1024)
    share_gpu = _bool_env("SEED_HOST_SHARE_GPU", False)
    shared_gpu_id = _optional_env("SEED_HOST_SHARED_GPU_ID")

    db = get_session_factory()()
    try:
        existing = db.scalars(select(Host).where(Host.display_name == display_name)).first()
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
                agent_url=agent_url,
                status=HostStatus.APPROVED,
                last_seen_at=datetime.now(timezone.utc),
            )
            db.add(host)
            db.commit()
            db.refresh(host)
            return host

        existing.display_name = display_name
        existing.kasm_url = kasm_url
        existing.kasm_token = kasm_token
        existing.agent_url = agent_url
        existing.shared_logical_cores = shared_logical_cores
        existing.shared_memory_bytes = shared_memory_bytes
        existing.share_gpu = share_gpu
        existing.shared_gpu_id = shared_gpu_id
        existing.lab_tier = LabTier.STANDARD
        existing.status = HostStatus.APPROVED
        existing.last_seen_at = datetime.now(timezone.utc)
        db.commit()
        db.refresh(existing)
        return existing
    finally:
        db.close()


def approve_seed_host(host_id: UUID) -> Host:
    load_env_files()
    get_settings()
    db = get_session_factory()()
    try:
        host = approve_host(db, host_id)
        if host is None:
            raise RuntimeError(f"host {host_id} not found")
        return host
    finally:
        db.close()


def main() -> None:
    parser = argparse.ArgumentParser(description="Seed or approve a Matchmaker host")
    parser.add_argument(
        "--approve-host-id",
        help="Set an existing host to approved (manual onboarding). Does not create a host.",
    )
    args = parser.parse_args()
    try:
        if args.approve_host_id:
            host = approve_seed_host(UUID(args.approve_host_id))
            action = "approved"
        else:
            host = seed_host()
            action = "seeded"
    except Exception as exc:
        print(f"seed failed: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
    token_note = "present" if host.kasm_token else "absent"
    print(
        f"{action} host {host.id} ({host.display_name}) status={host.status} "
        f"kasmUrl={host.kasm_url} kasmToken={token_note}"
    )


if __name__ == "__main__":
    main()
