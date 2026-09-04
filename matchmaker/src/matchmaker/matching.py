"""Host selection. Pending and stale hosts are never booked; replace this class later."""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Protocol

from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.models import Host, HostStatus, LabSession, SessionStatus

# Architecture §10: ping every few seconds; ~30–60s of silence is stale.
# 45s is the midpoint. Occupancy stays on session indexes, not this cutoff.
HEARTBEAT_STALE_AFTER = timedelta(seconds=45)


class HostSelector(Protocol):
    def select(self, db: Session, *, lab_tier: str) -> Host | None:
        """Return a host that can take a new session, or None if none are free."""


def heartbeat_freshness_cutoff(now: datetime | None = None) -> datetime:
    """Hosts with last_seen_at before this instant are too stale to book."""
    moment = now if now is not None else datetime.now(timezone.utc)
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=timezone.utc)
    return moment - HEARTBEAT_STALE_AFTER


def heartbeat_is_fresh(last_seen_at: datetime | None, *, now: datetime | None = None) -> bool:
    """Same rule the selector applies in SQL. Never-seen hosts are not live."""
    if last_seen_at is None:
        return False
    seen = last_seen_at if last_seen_at.tzinfo is not None else last_seen_at.replace(tzinfo=timezone.utc)
    return seen >= heartbeat_freshness_cutoff(now)


class SingleApprovedHostSelector:
    """Matcher: an approved host with a fresh heartbeat and no active session.

    Pending hosts are skipped. Last-seen older than HEARTBEAT_STALE_AFTER, or
    never pinged, is skipped here — not in the session router.

    Later this is where ranking belongs: filter by lab tier and inventory, then
    score remaining hosts by latency (critical for Pro) and reliability. Do not
    inline that logic into the session router — swap this class instead.
    """

    def select(self, db: Session, *, lab_tier: str) -> Host | None:
        occupied_host_ids = select(LabSession.host_id).where(
            LabSession.status == SessionStatus.ACTIVE
        )
        cutoff = heartbeat_freshness_cutoff()
        stmt = (
            select(Host)
            .where(Host.status == HostStatus.APPROVED)
            .where(Host.lab_tier == lab_tier)
            .where(Host.id.not_in(occupied_host_ids))
            .where(Host.last_seen_at >= cutoff)
            .limit(1)
        )
        return db.scalars(stmt).first()
