"""Host selection. Pending hosts are never booked; replace this class later."""

from __future__ import annotations

from typing import Protocol

from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.models import Host, HostStatus, LabSession, SessionStatus


class HostSelector(Protocol):
    def select(self, db: Session, *, lab_tier: str) -> Host | None:
        """Return a host that can take a new session, or None if none are free."""


class SingleApprovedHostSelector:
    """Matcher: an approved host with no active session. Pending hosts are skipped.

    Later this is where ranking belongs: filter by lab tier and inventory, then
    score remaining hosts by latency (critical for Pro) and reliability. Do not
    inline that logic into the session router — swap this class instead.
    """

    def select(self, db: Session, *, lab_tier: str) -> Host | None:
        occupied_host_ids = select(LabSession.host_id).where(
            LabSession.status == SessionStatus.ACTIVE
        )
        stmt = (
            select(Host)
            .where(Host.status == HostStatus.APPROVED)
            .where(Host.lab_tier == lab_tier)
            .where(Host.id.not_in(occupied_host_ids))
            .limit(1)
        )
        return db.scalars(stmt).first()
