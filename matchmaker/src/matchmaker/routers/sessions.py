from __future__ import annotations

from datetime import datetime, timezone
from typing import Annotated
from uuid import UUID

from fastapi import APIRouter, Depends, HTTPException, Response, status
from sqlalchemy import select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from matchmaker.db import get_db
from matchmaker.deps import get_current_user, get_host_provisioner, get_host_selector
from matchmaker.matching import HostSelector
from matchmaker.models import Host, LabSession, SessionStatus, User
from matchmaker.provision import HostProvisioner, ProvisionResult
from matchmaker.schemas import (
    ConnectionInfo,
    EndSessionResponse,
    SessionRequest,
    SessionResponse,
)

router = APIRouter()


def _session_response(session: LabSession, host: Host) -> SessionResponse:
    return SessionResponse(
        session_id=session.id,
        host_id=host.id,
        lab_tier=host.lab_tier,
        started_at=session.started_at,
        connection=ConnectionInfo(kasm_url=host.kasm_url, kasm_token=host.kasm_token),
    )


def _apply_provision_result(db: Session, host: Host, result: ProvisionResult) -> None:
    """Keep existing-session 200 on the same connection URL without a new field."""
    host.kasm_url = result.connection_url
    if result.connection_token is not None:
        host.kasm_token = result.connection_token
    db.commit()
    db.refresh(host)


def _provision_new_session(
    db: Session,
    host: Host,
    provisioner: HostProvisioner,
    *,
    lab_tier: str,
) -> None:
    result = provisioner.provision(host, lab_tier=lab_tier)
    if result is None or not result.connection_url.strip():
        return
    _apply_provision_result(db, host, result)


def _active_session_for_user(db: Session, user_id: UUID) -> LabSession | None:
    return db.scalars(
        select(LabSession).where(
            LabSession.user_id == user_id,
            LabSession.status == SessionStatus.ACTIVE,
        )
    ).first()


def _host_or_error(db: Session, host_id: UUID) -> Host:
    host = db.get(Host, host_id)
    if host is None:
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail="Session host is missing",
        )
    return host


@router.post("/sessions", response_model=SessionResponse)
def request_session(
    response: Response,
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
    selector: Annotated[HostSelector, Depends(get_host_selector)],
    provisioner: Annotated[HostProvisioner, Depends(get_host_provisioner)],
    payload: SessionRequest | None = None,
) -> SessionResponse:
    req = payload or SessionRequest()

    existing = _active_session_for_user(db, user.id)
    if existing is not None:
        response.status_code = status.HTTP_200_OK
        return _session_response(existing, _host_or_error(db, existing.host_id))

    # --- later plug-ins (do not implement here) ---
    # 1. Real matching: swap the HostSelector implementation for latency/reliability ranking.
    # 2. Headscale/Tailscale: mint a per-session mesh identity and scoped ACLs.
    # 3. Storj/rclone: mount the user's bucket inside the guest.
    # 4. Billing: start metering once the session is actually ready.
    host = selector.select(db, lab_tier=req.lab_tier)
    if host is None:
        raise HTTPException(
            status_code=status.HTTP_409_CONFLICT,
            detail="Host unavailable",
        )

    session = LabSession(
        user_id=user.id,
        host_id=host.id,
        status=SessionStatus.ACTIVE,
        started_at=datetime.now(timezone.utc),
    )
    db.add(session)
    try:
        db.commit()
    except IntegrityError:
        db.rollback()
        raced = _active_session_for_user(db, user.id)
        if raced is not None:
            response.status_code = status.HTTP_200_OK
            return _session_response(raced, _host_or_error(db, raced.host_id))
        raise HTTPException(
            status_code=status.HTTP_409_CONFLICT,
            detail="Host unavailable",
        ) from None

    db.refresh(session)
    # Occupancy is already held by the session row. Provision is a port on the
    # selected host (HTTP to its agent, or a test double). If the agent is
    # unreachable, keep the seeded/placeholder URL on the host row.
    _provision_new_session(db, host, provisioner, lab_tier=req.lab_tier)
    response.status_code = status.HTTP_201_CREATED
    return _session_response(session, host)


@router.post("/sessions/{session_id}/end", response_model=EndSessionResponse)
def end_session(
    session_id: UUID,
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
) -> EndSessionResponse:
    session = db.get(LabSession, session_id)
    if session is None or session.user_id != user.id:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Session not found")

    if session.status != SessionStatus.ENDED:
        session.status = SessionStatus.ENDED
        session.ended_at = datetime.now(timezone.utc)
        db.commit()
        db.refresh(session)

    if session.ended_at is None:
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail="Ended session is missing endedAt",
        )

    return EndSessionResponse(
        session_id=session.id,
        status=session.status,
        ended_at=session.ended_at,
    )
