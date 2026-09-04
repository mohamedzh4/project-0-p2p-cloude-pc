from __future__ import annotations

from typing import Annotated
from uuid import UUID

from fastapi import APIRouter, Depends, HTTPException, Response, status
from sqlalchemy.orm import Session

from matchmaker.db import get_db
from matchmaker.deps import require_host_approver, require_host_bootstrap
from matchmaker.inventory import approve_host, apply_declared_caps, host_response, new_pending_host
from matchmaker.models import Host
from matchmaker.schemas import HostInventoryRequest, HostResponse

router = APIRouter()


@router.post("/hosts", response_model=HostResponse, status_code=status.HTTP_201_CREATED)
def register_host(
    payload: HostInventoryRequest,
    db: Annotated[Session, Depends(get_db)],
    _: Annotated[None, Depends(require_host_bootstrap)],
) -> HostResponse:
    host = new_pending_host(payload)
    db.add(host)
    db.commit()
    db.refresh(host)
    return host_response(host)


@router.put("/hosts/{host_id}", response_model=HostResponse)
def update_host(
    host_id: UUID,
    payload: HostInventoryRequest,
    response: Response,
    db: Annotated[Session, Depends(get_db)],
    _: Annotated[None, Depends(require_host_bootstrap)],
) -> HostResponse:
    existing = db.get(Host, host_id)
    if existing is None:
        host = new_pending_host(payload, host_id=host_id)
        db.add(host)
        db.commit()
        db.refresh(host)
        response.status_code = status.HTTP_201_CREATED
        return host_response(host)

    apply_declared_caps(
        existing,
        payload,
        replace_kasm_url="kasm_url" in payload.model_fields_set,
    )
    db.commit()
    db.refresh(existing)
    response.status_code = status.HTTP_200_OK
    return host_response(existing)


@router.post("/hosts/{host_id}/approve", response_model=HostResponse)
def approve_registered_host(
    host_id: UUID,
    db: Annotated[Session, Depends(get_db)],
    _: Annotated[None, Depends(require_host_approver)],
) -> HostResponse:
    host = approve_host(db, host_id)
    if host is None:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Host not found")
    return host_response(host)
