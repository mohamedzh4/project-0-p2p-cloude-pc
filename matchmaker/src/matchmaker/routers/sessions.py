from __future__ import annotations

from datetime import datetime, timezone
from typing import Annotated
from uuid import UUID

from fastapi import APIRouter, Depends, HTTPException, Response, status
from sqlalchemy import select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from matchmaker.booking import (
    ProvisionScheduler,
    occupying_session_for_user,
    open_session,
    provision_job_runner,
    release_session_grants,
)
from matchmaker.config import get_settings
from matchmaker.db import get_db, get_session_factory
from matchmaker.deps import (
    get_current_user,
    get_headscale_client,
    get_host_provisioner,
    get_host_selector,
    get_provision_scheduler,
    get_renterd_client,
)
from matchmaker.headscale import HeadscaleClient, HeadscaleError
from matchmaker.ingress import connection_for_browser
from matchmaker.ledger import (
    debit_amounts_for_sessions,
    duration_seconds,
    settle_ended_session,
    user_credits_debited,
    user_debit_for_session,
)
from matchmaker.matching import HostSelector
from matchmaker.mesh import session_mesh_info
from matchmaker.models import Host, LabSession, SessionStatus, User, UserStorage
from matchmaker.provision import (
    HostProvisioner,
    session_connection,
    teardown_booked_host,
)
from matchmaker.renterd import RenterdClient, RenterdError, StorageGrant
from matchmaker.schemas import (
    EndSessionResponse,
    SessionHistoryItem,
    SessionHistoryResponse,
    SessionRequest,
    SessionResponse,
)
from matchmaker.silence import reclaim_stale_sessions
from matchmaker.storage import (
    ensure_user_storage,
    locators_from_grant,
    mount_info_from_grant,
    storage_grant,
)

router = APIRouter()


def _session_response(
    db: Session,
    session: LabSession,
    host: Host,
    grant: StorageGrant,
) -> SessionResponse:
    settings = get_settings()
    connection = None
    if session.status == SessionStatus.ACTIVE:
        connection = connection_for_browser(
            db,
            session,
            session_connection(host),
            public_origin=settings.gateway_public_origin,
            ttl_seconds=settings.gateway_token_ttl_seconds,
        )
    return SessionResponse(
        session_id=session.id,
        host_id=host.id,
        lab_tier=host.lab_tier,
        status=session.status,
        failure_reason=session.failure_reason,
        started_at=session.started_at,
        ended_at=session.ended_at,
        connection=connection,
        storage=mount_info_from_grant(grant, saved_as_of=session.saved_as_of),
        mesh=session_mesh_info(
            session.id,
            login_server=settings.headscale_login_server,
            exit_node=settings.headscale_exit_node,
        ),
    )


def _ensure_storage(db: Session, user: User, renterd: RenterdClient) -> StorageGrant:
    settings = get_settings()
    try:
        return ensure_user_storage(
            db,
            user,
            renterd,
            endpoint=settings.renterd_s3_guest_endpoint,
            region=settings.renterd_s3_region,
            shared_bucket=settings.storage_shared_bucket,
        )
    except RenterdError as exc:
        raise HTTPException(
            status_code=status.HTTP_502_BAD_GATEWAY,
            detail="renterd storage provision failed",
        ) from exc


def _recorded_storage(db: Session, user: User) -> StorageGrant:
    """The user's storage locators from the database. No storage call."""
    settings = get_settings()
    row = db.get(UserStorage, user.id)
    return storage_grant(
        user_id=user.id,
        endpoint=settings.renterd_s3_guest_endpoint,
        region=settings.renterd_s3_region,
        bucket=row.bucket if row is not None else None,
        **({"prefix": row.prefix} if row is not None else {}),
    )


def _reclaim_stale(
    db: Session,
    selector: HostSelector,
    provisioner: HostProvisioner,
    headscale: HeadscaleClient,
    scheduler: ProvisionScheduler,
) -> None:
    reclaim_stale_sessions(
        db,
        selector,
        provisioner,
        credits_per_minute=get_settings().credits_per_minute,
        headscale=headscale,
        scheduler=scheduler,
    )


def _host_or_error(db: Session, host_id: UUID) -> Host:
    host = db.get(Host, host_id)
    if host is None:
        raise HTTPException(
            status_code=status.HTTP_500_INTERNAL_SERVER_ERROR,
            detail="Session host is missing",
        )
    return host


def _status_code_for(session: LabSession) -> int:
    if session.status == SessionStatus.PROVISIONING:
        return status.HTTP_202_ACCEPTED
    return status.HTTP_200_OK


@router.post("/sessions", response_model=SessionResponse)
def request_session(
    response: Response,
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
    selector: Annotated[HostSelector, Depends(get_host_selector)],
    provisioner: Annotated[HostProvisioner, Depends(get_host_provisioner)],
    renterd: Annotated[RenterdClient, Depends(get_renterd_client)],
    headscale: Annotated[HeadscaleClient, Depends(get_headscale_client)],
    scheduler: Annotated[ProvisionScheduler, Depends(get_provision_scheduler)],
    payload: SessionRequest | None = None,
) -> SessionResponse:
    """Book a host. Returns at once; the desktop boots in the background.

    201: the desktop is ready (`connection` is set).
    202: `provisioning`; poll `GET /v1/sessions/{id}`.
    200: this user already has a session; it is returned as it is now.
    409: no host is free. 502: the host failed before this call returned.
    """
    req = payload or SessionRequest()
    _reclaim_stale(db, selector, provisioner, headscale, scheduler)

    existing = occupying_session_for_user(db, user.id)
    if existing is not None:
        response.status_code = _status_code_for(existing)
        return _session_response(
            db, existing, _host_or_error(db, existing.host_id), _recorded_storage(db, user)
        )

    # Matching: ranking lives in the injected HostSelector. Storage: the
    # user's bucket/prefix is ensured before the host is booked and follows
    # the user across hosts. Billing: metered from desktop-ready to end.
    grant = _ensure_storage(db, user, renterd)
    host = selector.select(db, lab_tier=req.lab_tier)
    if host is None:
        raise HTTPException(
            status_code=status.HTTP_409_CONFLICT,
            detail="Host unavailable",
        )

    try:
        opened = open_session(
            db,
            user_id=user.id,
            host=host,
            lab_tier=req.lab_tier,
            headscale=headscale,
            storage=locators_from_grant(grant),
        )
    except HeadscaleError as exc:
        raise HTTPException(
            status_code=status.HTTP_502_BAD_GATEWAY,
            detail="Headscale session mesh mint failed",
        ) from exc
    if opened is None:
        raced = occupying_session_for_user(db, user.id)
        if raced is not None:
            response.status_code = _status_code_for(raced)
            return _session_response(db, raced, _host_or_error(db, raced.host_id), grant)
        raise HTTPException(
            status_code=status.HTTP_409_CONFLICT,
            detail="Host unavailable",
        )

    session, job = opened
    scheduler.submit(
        provision_job_runner(
            job,
            provisioner=provisioner,
            headscale=headscale,
            session_factory=get_session_factory(),
        )
    )
    db.refresh(session)
    db.refresh(host)
    if session.status == SessionStatus.FAILED:
        raise HTTPException(
            status_code=status.HTTP_502_BAD_GATEWAY,
            detail="host provision failed",
        )
    response.status_code = (
        status.HTTP_201_CREATED
        if session.status == SessionStatus.ACTIVE
        else status.HTTP_202_ACCEPTED
    )
    return _session_response(db, session, host, grant)


@router.get("/sessions/current", response_model=SessionResponse)
def current_session(
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
) -> SessionResponse:
    """The session this user holds now (booting or ready). 404 when none."""
    session = occupying_session_for_user(db, user.id)
    if session is None:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Session not found")
    return _session_response(
        db, session, _host_or_error(db, session.host_id), _recorded_storage(db, user)
    )


@router.get("/sessions/{session_id}", response_model=SessionResponse)
def get_session(
    session_id: UUID,
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
) -> SessionResponse:
    """One of this user's sessions, in its current state. Poll while provisioning."""
    session = db.get(LabSession, session_id)
    if session is None or session.user_id != user.id:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Session not found")
    return _session_response(
        db, session, _host_or_error(db, session.host_id), _recorded_storage(db, user)
    )


@router.post("/sessions/{session_id}/end", response_model=EndSessionResponse)
def end_session(
    session_id: UUID,
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
    headscale: Annotated[HeadscaleClient, Depends(get_headscale_client)],
    provisioner: Annotated[HostProvisioner, Depends(get_host_provisioner)],
) -> EndSessionResponse:
    session = db.get(LabSession, session_id)
    if session is None or session.user_id != user.id:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Session not found")

    if session.status in (SessionStatus.ACTIVE, SessionStatus.PROVISIONING):
        was_active = session.status == SessionStatus.ACTIVE
        session.status = SessionStatus.ENDED
        session.ended_at = datetime.now(timezone.utc)
        if was_active:
            # A desktop that never became ready is not billed.
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
        host = db.get(Host, session.host_id)
        teardown_booked_host(provisioner, host)
        release_session_grants(db, session.id, headscale=headscale)

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
    selector: Annotated[HostSelector, Depends(get_host_selector)],
    provisioner: Annotated[HostProvisioner, Depends(get_host_provisioner)],
    headscale: Annotated[HeadscaleClient, Depends(get_headscale_client)],
    scheduler: Annotated[ProvisionScheduler, Depends(get_provision_scheduler)],
) -> SessionHistoryResponse:
    _reclaim_stale(db, selector, provisioner, headscale, scheduler)
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
                failure_reason=row.failure_reason,
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
