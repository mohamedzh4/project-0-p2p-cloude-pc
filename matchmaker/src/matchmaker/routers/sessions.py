from __future__ import annotations

from datetime import datetime, timezone
from typing import Annotated
from uuid import UUID

from fastapi import APIRouter, Depends, HTTPException, Response, status
from sqlalchemy import select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from matchmaker.config import get_settings
from matchmaker.db import get_db
from matchmaker.deps import get_current_user, get_host_provisioner, get_host_selector
from matchmaker.ledger import (
    debit_amounts_for_sessions,
    duration_seconds,
    settle_ended_session,
    user_credits_debited,
    user_debit_for_session,
)
from matchmaker.matching import HostSelector
from matchmaker.models import Host, LabSession, SessionStatus, User
from matchmaker.provision import HostProvisioner, ProvisionResult
from matchmaker.schemas import (
    ConnectionInfo,
    EndSessionResponse,
    SessionHistoryItem,
    SessionHistoryResponse,
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
    # 1. Matching: ranking lives in the HostSelector implementation (declared
    #    caps today; latency/reliability when those can be measured). Do not
    #    inline it here — swap the injected class.
    # 2. Headscale/Tailscale: mint a per-session mesh identity and scoped ACLs.
    # 3. Storj/rclone: mount the user's bucket inside the guest.
    # 4. Billing: duration is metered at end (startedAt → endedAt) into
    #    ledger_entries. Occupancy start is the clock; no payment processor.
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
        settle_ended_session(
            db,
            session,
            credits_per_minute=get_settings().credits_per_minute,
        )
        try:
            db.commit()
        except IntegrityError:
            db.rollback()
            session = db.get(LabSession, session_id)
            if session is None or session.user_id != user.id:
                raise HTTPException(
                    status_code=status.HTTP_404_NOT_FOUND, detail="Session not found"
                ) from None
        else:
            db.refresh(session)

    if session.ended_at is None:
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail="Ended session is missing endedAt",
        )

    debited = user_debit_for_session(db, session.id)
    return EndSessionResponse(
        session_id=session.id,
        status=session.status,
        ended_at=session.ended_at,
        credits_debited=debited if debited is not None else 0,
    )


@router.get("/sessions", response_model=SessionHistoryResponse)
def list_sessions(
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
) -> SessionHistoryResponse:
    rows = db.scalars(
        select(LabSession)
        .where(LabSession.user_id == user.id)
        .order_by(LabSession.started_at.desc())
    ).all()
    debits = debit_amounts_for_sessions(db, [row.id for row in rows])
    items: list[SessionHistoryItem] = []
    for row in rows:
        host = _host_or_error(db, row.host_id)
        ended_at = row.ended_at
        items.append(
            SessionHistoryItem(
                session_id=row.id,
                host_id=row.host_id,
                lab_tier=host.lab_tier,
                status=row.status,
                started_at=row.started_at,
                ended_at=ended_at,
                duration_seconds=(
                    duration_seconds(row.started_at, ended_at) if ended_at is not None else None
                ),
                credits_debited=debits.get(row.id, 0),
            )
        )
    return SessionHistoryResponse(
        sessions=items,
        credits_debited=user_credits_debited(db, user.id),
    )
