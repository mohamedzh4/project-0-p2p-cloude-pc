"""Declared host caps. Occupancy stays on session indexes, not a host flag."""

from __future__ import annotations

from datetime import datetime, timezone
from uuid import UUID

from sqlalchemy.orm import Session

from matchmaker.models import Host, HostStatus
from matchmaker.schemas import HostInventoryRequest, HostResponse


def kasm_url_for_storage(value: str | None) -> str:
    return value or ""


def host_response(host: Host) -> HostResponse:
    return HostResponse(
        host_id=host.id,
        display_name=host.display_name,
        lab_tier=host.lab_tier,
        status=host.status,
        shared_logical_cores=host.shared_logical_cores,
        shared_memory_bytes=host.shared_memory_bytes,
        share_gpu=host.share_gpu,
        shared_gpu_id=host.shared_gpu_id,
        kasm_url=host.kasm_url or None,
        last_seen_at=host.last_seen_at,
    )


def apply_declared_caps(
    host: Host,
    payload: HostInventoryRequest,
    *,
    replace_kasm_url: bool,
) -> None:
    host.display_name = payload.display_name
    host.lab_tier = payload.lab_tier
    host.shared_logical_cores = payload.shared_logical_cores
    host.shared_memory_bytes = payload.shared_memory_bytes
    host.share_gpu = payload.share_gpu
    host.shared_gpu_id = payload.shared_gpu_id
    if replace_kasm_url:
        host.kasm_url = kasm_url_for_storage(payload.kasm_url)


def new_pending_host(
    payload: HostInventoryRequest,
    *,
    host_id: UUID | None = None,
) -> Host:
    host = Host(
        display_name=payload.display_name,
        lab_tier=payload.lab_tier,
        shared_logical_cores=payload.shared_logical_cores,
        shared_memory_bytes=payload.shared_memory_bytes,
        share_gpu=payload.share_gpu,
        shared_gpu_id=payload.shared_gpu_id,
        kasm_url=kasm_url_for_storage(payload.kasm_url),
        kasm_token=None,
        status=HostStatus.PENDING,
    )
    if host_id is not None:
        host.id = host_id
    return host


def approve_host(db: Session, host_id: UUID) -> Host | None:
    host = db.get(Host, host_id)
    if host is None:
        return None
    host.status = HostStatus.APPROVED
    db.commit()
    db.refresh(host)
    return host


def record_heartbeat(db: Session, host_id: UUID) -> Host | None:
    """Stamp last-seen. Does not approve, change caps, or touch occupancy."""
    host = db.get(Host, host_id)
    if host is None:
        return None
    host.last_seen_at = datetime.now(timezone.utc)
    db.commit()
    db.refresh(host)
    return host
