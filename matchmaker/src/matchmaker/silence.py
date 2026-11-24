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

from matchmaker.booking import (
    InlineScheduler,
    ProvisionScheduler,
    fail_stuck_provisioning,
    occupying_session_for_user,
    open_session,
    provision_job_runner,
    release_session_grants,
)
from matchmaker.config import get_settings
from matchmaker.db import get_session_factory
from matchmaker.headscale import HeadscaleClient, HeadscaleError
from matchmaker.ledger import settle_ended_session
from matchmaker.matching import HostSelector, heartbeat_freshness_cutoff
from matchmaker.models import Host, LabSession, SessionStatus, UserStorage
from matchmaker.provision import HostProvisioner, teardown_booked_host
from matchmaker.storage import GuestStorageLocators


def reclaim_stale_sessions(
    db: Session,
    selector: HostSelector,
    provisioner: HostProvisioner,
    *,
    credits_per_minute: int,
    now: datetime | None = None,
    headscale: HeadscaleClient | None = None,
    scheduler: ProvisionScheduler | None = None,
) -> list[UUID]:
    """Interrupt stale occupied sessions, then try one rematch each.

    Interrupted is not `active`, so the partial unique indexes free occupancy.
    If select returns None, the user has no active session (409 on next book).
    Returns ids of sessions this sweep closed as interrupted.
    """
    moment = _as_utc(now)
    fail_stuck_provisioning(
        db,
        timeout_seconds=get_settings().provision_timeout_seconds,
        headscale=headscale,
        provisioner=provisioner,
        now=moment,
    )
    interrupted_ids: list[UUID] = []
    for session, lab_tier in _stale_active_rows(db, now=moment):
        session_id = session.id
        user_id = session.user_id
        old_host = db.get(Host, session.host_id)
        if not _interrupt_stale_session(
            db,
            session,
            credits_per_minute=credits_per_minute,
            now=moment,
        ):
            continue
        interrupted_ids.append(session_id)
        teardown_booked_host(provisioner, old_host)
        release_session_grants(db, session_id, headscale=headscale)
        _rematch_user(
            db,
            user_id=user_id,
            lab_tier=lab_tier,
            selector=selector,
            provisioner=provisioner,
            headscale=headscale,
            scheduler=scheduler,
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
    scheduler: ProvisionScheduler | None = None,
) -> LabSession | None:
    """Book another host for an interrupted user. The boot runs on `scheduler`."""
    existing = occupying_session_for_user(db, user_id)
    if existing is not None:
        return existing

    host = selector.select(db, lab_tier=lab_tier)
    if host is None:
        return None

    try:
        opened = open_session(
            db,
            user_id=user_id,
            host=host,
            lab_tier=lab_tier,
            headscale=headscale,
            storage=_storage_locators_for_user(db, user_id),
        )
    except HeadscaleError:
        return None
    if opened is None:
        return occupying_session_for_user(db, user_id)
    session, job = opened
    (scheduler or InlineScheduler()).submit(
        provision_job_runner(
            job,
            provisioner=provisioner,
            headscale=headscale,
            session_factory=get_session_factory(),
        )
    )
    db.refresh(session)
    if session.status == SessionStatus.FAILED:
        return None
    return session


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
