"""Heartbeat-silence teardown. Occupancy stays on session indexes.

Active sessions on hosts past HEARTBEAT_STALE_AFTER are marked interrupted
and settled on the ledger (same meter as a user end). Rematch is one
HostSelector.select call — ranking stays in the injected selector.
"""

from __future__ import annotations

from datetime import datetime, timezone
from uuid import UUID, uuid4

from sqlalchemy import or_, select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from matchmaker.ledger import settle_ended_session
from matchmaker.matching import HostSelector, heartbeat_freshness_cutoff
from matchmaker.config import get_settings
from matchmaker.headscale import HeadscaleClient, HeadscaleError
from matchmaker.mesh import expire_session_mesh, guest_mesh_hint, mint_session_mesh
from matchmaker.models import Host, LabSession, SessionStatus, UserStorage
from matchmaker.provision import HostProvisioner, provision_booked_host
from matchmaker.storage import GuestStorageLocators


def reclaim_stale_sessions(
    db: Session,
    selector: HostSelector,
    provisioner: HostProvisioner,
    *,
    credits_per_minute: int,
    now: datetime | None = None,
    headscale: HeadscaleClient | None = None,
) -> list[UUID]:
    """Interrupt stale occupied sessions, then try one rematch each.

    Interrupted is not `active`, so the partial unique indexes free occupancy.
    If select returns None, the user has no active session (409 on next book).
    Returns ids of sessions this sweep closed as interrupted.
    """
    moment = _as_utc(now)
    interrupted_ids: list[UUID] = []
    for session, lab_tier in _stale_active_rows(db, now=moment):
        session_id = session.id
        user_id = session.user_id
        if not _interrupt_stale_session(
            db,
            session,
            credits_per_minute=credits_per_minute,
            now=moment,
        ):
            continue
        interrupted_ids.append(session_id)
        if headscale is not None:
            try:
                expire_session_mesh(headscale, session_id=session_id)
            except HeadscaleError:
                pass
        _rematch_user(
            db,
            user_id=user_id,
            lab_tier=lab_tier,
            selector=selector,
            provisioner=provisioner,
            headscale=headscale,
        )
    return interrupted_ids


def _stale_active_rows(db: Session, *, now: datetime) -> list[tuple[LabSession, str]]:
    cutoff = heartbeat_freshness_cutoff(now)
    stmt = (
        select(LabSession, Host.lab_tier)
        .join(Host, Host.id == LabSession.host_id)
        .where(LabSession.status == SessionStatus.ACTIVE)
        .where(or_(Host.last_seen_at.is_(None), Host.last_seen_at < cutoff))
    )
    return [(session, lab_tier) for session, lab_tier in db.execute(stmt).all()]


def _interrupt_stale_session(
    db: Session,
    session: LabSession,
    *,
    credits_per_minute: int,
    now: datetime,
) -> bool:
    """Return True when the session is interrupted (this call or a concurrent sweep).

    User-ended sessions stay ended and must not rematch.
    """
    current = db.get(LabSession, session.id)
    if current is None:
        return False
    if current.status == SessionStatus.INTERRUPTED:
        return True
    if current.status != SessionStatus.ACTIVE:
        return False

    current.status = SessionStatus.INTERRUPTED
    current.ended_at = now
    settle_ended_session(db, current, credits_per_minute=credits_per_minute)
    try:
        db.commit()
    except IntegrityError:
        db.rollback()
        current = db.get(LabSession, session.id)
        if current is None:
            return False
        return current.status == SessionStatus.INTERRUPTED
    db.refresh(current)
    return current.status == SessionStatus.INTERRUPTED


def _rematch_user(
    db: Session,
    *,
    user_id: UUID,
    lab_tier: str,
    selector: HostSelector,
    provisioner: HostProvisioner,
    headscale: HeadscaleClient | None = None,
) -> LabSession | None:
    existing = _active_session_for_user(db, user_id)
    if existing is not None:
        return existing

    host = selector.select(db, lab_tier=lab_tier)
    if host is None:
        return None

    session_id = uuid4()
    settings = get_settings()
    mesh = None
    if headscale is not None:
        try:
            mesh = mint_session_mesh(
                headscale,
                session_id=session_id,
                login_server=settings.headscale_login_server,
                exit_node=settings.headscale_exit_node,
            )
        except HeadscaleError:
            return None

    session = LabSession(
        id=session_id,
        user_id=user_id,
        host_id=host.id,
        status=SessionStatus.ACTIVE,
        started_at=datetime.now(timezone.utc),
    )
    db.add(session)
    try:
        db.commit()
    except IntegrityError:
        db.rollback()
        return _active_session_for_user(db, user_id)

    db.refresh(session)
    storage = _storage_locators_for_user(db, user_id)
    session_id = session.id
    if not provision_booked_host(
        db,
        host,
        provisioner,
        lab_tier=lab_tier,
        storage=storage,
        guest_mesh=guest_mesh_hint(mesh) if mesh is not None else None,
    ):
        db.delete(session)
        db.commit()
        if headscale is not None:
            try:
                expire_session_mesh(headscale, session_id=session_id)
            except HeadscaleError:
                pass
        return None
    return session


def _active_session_for_user(db: Session, user_id: UUID) -> LabSession | None:
    return db.scalars(
        select(LabSession).where(
            LabSession.user_id == user_id,
            LabSession.status == SessionStatus.ACTIVE,
        )
    ).first()


def _storage_locators_for_user(db: Session, user_id: UUID) -> GuestStorageLocators | None:
    row = db.get(UserStorage, user_id)
    if row is None:
        return None
    settings = get_settings()
    return GuestStorageLocators(
        endpoint=settings.renterd_s3_guest_endpoint,
        bucket=row.bucket,
        prefix=row.prefix,
        region=settings.renterd_s3_region,
    )


def _as_utc(now: datetime | None) -> datetime:
    moment = now if now is not None else datetime.now(timezone.utc)
    if moment.tzinfo is None:
        return moment.replace(tzinfo=timezone.utc)
    return moment.astimezone(timezone.utc)
