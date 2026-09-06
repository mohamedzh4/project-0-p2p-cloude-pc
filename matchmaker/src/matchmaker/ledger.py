"""Internal session meter. Credits are ledger rows, not a payment processor."""

from __future__ import annotations

from datetime import datetime, timezone
from uuid import UUID

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from matchmaker.models import (
    LabSession,
    LedgerDirection,
    LedgerEntry,
    LedgerParty,
)


def duration_seconds(started_at: datetime, ended_at: datetime) -> int:
    start = _as_utc(started_at)
    end = _as_utc(ended_at)
    return max(0, int((end - start).total_seconds()))


def billed_minutes(duration_secs: int) -> int:
    """Whole minutes, rounding up. A zero-length session bills nothing."""
    if duration_secs <= 0:
        return 0
    return (duration_secs + 59) // 60


def credits_for_duration(
    started_at: datetime,
    ended_at: datetime,
    credits_per_minute: int,
) -> tuple[int, int]:
    """Return (duration_seconds, amount_credits) for a closed interval."""
    seconds = duration_seconds(started_at, ended_at)
    return seconds, billed_minutes(seconds) * credits_per_minute


def settle_ended_session(
    db: Session,
    session: LabSession,
    *,
    credits_per_minute: int,
) -> int:
    """Write user debit + host credit. Idempotent per session. Caller commits."""
    if session.ended_at is None:
        raise ValueError("ended_at is required to settle a session")

    existing = user_debit_for_session(db, session.id)
    if existing is not None:
        return existing

    seconds, amount = credits_for_duration(
        session.started_at,
        session.ended_at,
        credits_per_minute,
    )
    db.add(
        LedgerEntry(
            session_id=session.id,
            user_id=session.user_id,
            host_id=None,
            party=LedgerParty.USER,
            direction=LedgerDirection.DEBIT,
            amount_credits=amount,
            duration_seconds=seconds,
            rate_credits_per_minute=credits_per_minute,
        )
    )
    db.add(
        LedgerEntry(
            session_id=session.id,
            user_id=None,
            host_id=session.host_id,
            party=LedgerParty.HOST,
            direction=LedgerDirection.CREDIT,
            amount_credits=amount,
            duration_seconds=seconds,
            rate_credits_per_minute=credits_per_minute,
        )
    )
    return amount


def user_debit_for_session(db: Session, session_id: UUID) -> int | None:
    row = db.scalars(
        select(LedgerEntry).where(
            LedgerEntry.session_id == session_id,
            LedgerEntry.party == LedgerParty.USER,
        )
    ).first()
    if row is None:
        return None
    return int(row.amount_credits)


def user_credits_debited(db: Session, user_id: UUID) -> int:
    total = db.execute(
        select(func.coalesce(func.sum(LedgerEntry.amount_credits), 0)).where(
            LedgerEntry.user_id == user_id,
            LedgerEntry.party == LedgerParty.USER,
        )
    ).scalar_one()
    return int(total)


def host_credits_accrued(db: Session, host_id: UUID) -> int:
    total = db.execute(
        select(func.coalesce(func.sum(LedgerEntry.amount_credits), 0)).where(
            LedgerEntry.host_id == host_id,
            LedgerEntry.party == LedgerParty.HOST,
        )
    ).scalar_one()
    return int(total)


def debit_amounts_for_sessions(db: Session, session_ids: list[UUID]) -> dict[UUID, int]:
    if not session_ids:
        return {}
    rows = db.scalars(
        select(LedgerEntry).where(
            LedgerEntry.session_id.in_(session_ids),
            LedgerEntry.party == LedgerParty.USER,
        )
    )
    return {row.session_id: int(row.amount_credits) for row in rows}


def _as_utc(value: datetime) -> datetime:
    if value.tzinfo is None:
        return value.replace(tzinfo=timezone.utc)
    return value.astimezone(timezone.utc)
