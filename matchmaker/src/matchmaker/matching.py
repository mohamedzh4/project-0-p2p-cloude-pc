"""Host selection. Pending and stale hosts are never booked.

Ranking uses declared inventory on the host row (CPU, then RAM, then id).
Latency and reliability are not scored until those can be measured.
"""

from __future__ import annotations

import os
from datetime import datetime, timedelta, timezone
from typing import Protocol

from sqlalchemy import or_, select
from sqlalchemy.orm import Session

from matchmaker.models import Host, HostStatus, LabSession, LabTier, OCCUPYING_STATUSES

# Architecture §10: ping every few seconds; ~30–60s of silence is stale.
# 45s is the midpoint. Occupancy stays on session indexes, not this cutoff.
# New books skip stale hosts here; silence.py interrupts occupied stale hosts.
def _stale_after() -> timedelta:
    """45 s by default (agents ping every 10 s). `HEARTBEAT_STALE_SECONDS`
    shortens it for the integration test; do not lower it in production."""
    raw = os.environ.get("HEARTBEAT_STALE_SECONDS", "").strip()
    try:
        seconds = int(raw) if raw else 45
    except ValueError:
        seconds = 45
    return timedelta(seconds=max(1, seconds))


HEARTBEAT_STALE_AFTER = _stale_after()


# A pull-mode agent asks for commands every 2 s. One that has not asked for
# this long is starting up, busy stopping a guest, or gone: a booking would
# wait for a pickup that does not come (issue 30).
POLL_STALE_AFTER = timedelta(seconds=20)


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


class RankedHostSelector:
    """Matcher: approved, fresh, unoccupied hosts of the requested tier.

    Filter (unchanged from the single-host matcher): status approved, matching
    lab_tier, no active session, last_seen_at within HEARTBEAT_STALE_AFTER.
    Never-seen hosts are skipped. Occupancy is the session indexes, not a lock.

    Rank among remaining rows by declared caps already stored on the host:
    more shared_logical_cores, then more shared_memory_bytes, then host id
    ascending so two equal-cap hosts always yield the same winner.

    Pro additionally requires share_gpu and a non-empty shared_gpu_id. GPU
    assignment is exclusive for the session, not a concurrent slice. Standard
    does not rank on GPU.

    Do not inline this into the session router — inject via get_host_selector().
    """

    def select(self, db: Session, *, lab_tier: str) -> Host | None:
        occupied_host_ids = select(LabSession.host_id).where(
            LabSession.status.in_(OCCUPYING_STATUSES)
        )
        cutoff = heartbeat_freshness_cutoff()
        polled_cutoff = datetime.now(timezone.utc) - POLL_STALE_AFTER
        stmt = (
            select(Host)
            .where(Host.status == HostStatus.APPROVED)
            .where(Host.lab_tier == lab_tier)
            .where(Host.id.not_in(occupied_host_ids))
            .where(Host.last_seen_at >= cutoff)
            .where(or_(Host.pull_mode.is_(False), Host.last_polled_at >= polled_cutoff))
        )
        if lab_tier == LabTier.PRO:
            stmt = (
                stmt.where(Host.share_gpu.is_(True))
                .where(Host.shared_gpu_id.is_not(None))
                .where(Host.shared_gpu_id != "")
            )
        stmt = stmt.order_by(
            Host.shared_logical_cores.desc(),
            Host.shared_memory_bytes.desc(),
            Host.id.asc(),
        ).limit(1)
        return db.scalars(stmt).first()
