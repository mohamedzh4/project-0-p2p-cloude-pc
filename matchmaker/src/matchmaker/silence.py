"""Heartbeat-silence teardown. Occupancy stays on session indexes.

Active sessions on hosts past HEARTBEAT_STALE_AFTER are marked interrupted
and settled on the ledger (same meter as a user end). Rematch is one
HostSelector.select call — ranking stays in the injected selector.
"""

from __future__ import annotations

from datetime import datetime, timezone
from uuid import UUID

from sqlalchemy import or_, select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from matchmaker.ledger import settle_ended_session
from matchmaker.matching import HostSelector, heartbeat_freshness_cutoff
from matchmaker.models import Host, LabSession, SessionStatus
from matchmaker.provision import HostProvisioner, provision_booked_host


def reclaim_stale_sessions(
    db: Session,
    selector: HostSelector,
    provisioner: HostProvisioner,
    *,
    credits_per_minute: int,
    now: datetime | None = None,
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
        _rematch_user(
            db,
            user_id=user_id,
            lab_tier=lab_tier,
            selector=selector,
            provisioner=provisioner,
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
) -> LabSession | None:
    existing = _active_session_for_user(db, user_id)
    if existing is not None:
        return existing

    host = selector.select(db, lab_tier=lab_tier)
    if host is None:
        return None

    session = LabSession(
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
    provision_booked_host(db, host, provisioner, lab_tier=lab_tier)
    return session


def _active_session_for_user(db: Session, user_id: UUID) -> LabSession | None:
    return db.scalars(
        select(LabSession).where(
            LabSession.user_id == user_id,
            LabSession.status == SessionStatus.ACTIVE,
        )
    ).first()


def _as_utc(now: datetime | None) -> datetime:
    moment = now if now is not None else datetime.now(timezone.utc)
    if moment.tzinfo is None:
        return moment.replace(tzinfo=timezone.utc)
    return moment.astimezone(timezone.utc)
