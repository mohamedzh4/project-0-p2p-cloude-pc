from __future__ import annotations

from datetime import datetime, timedelta, timezone

from sqlalchemy.orm import Session

from matchmaker.auth import hash_password
from matchmaker.matching import (
    HEARTBEAT_STALE_AFTER,
    SingleApprovedHostSelector,
    heartbeat_freshness_cutoff,
    heartbeat_is_fresh,
)
from matchmaker.models import Host, HostStatus, LabSession, LabTier, SessionStatus, User


def test_heartbeat_cutoff_is_45_seconds() -> None:
    now = datetime(2026, 9, 4, 12, 0, 0, tzinfo=timezone.utc)
    assert HEARTBEAT_STALE_AFTER == timedelta(seconds=45)
    assert heartbeat_freshness_cutoff(now) == now - timedelta(seconds=45)


def test_never_seen_heartbeat_is_not_fresh() -> None:
    now = datetime(2026, 9, 4, 12, 0, 0, tzinfo=timezone.utc)
    assert heartbeat_is_fresh(None, now=now) is False


def test_heartbeat_at_cutoff_is_still_fresh() -> None:
    now = datetime(2026, 9, 4, 12, 0, 0, tzinfo=timezone.utc)
    cutoff = heartbeat_freshness_cutoff(now)
    assert heartbeat_is_fresh(cutoff, now=now) is True
    assert heartbeat_is_fresh(cutoff - timedelta(microseconds=1), now=now) is False


def test_selector_returns_the_approved_host_when_free(db: Session, seeded_host: Host) -> None:
    selected = SingleApprovedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == seeded_host.id


def test_selector_returns_none_when_host_is_occupied(db: Session, seeded_host: Host) -> None:
    user = User(email="guest@example.com", password_hash=hash_password("password123"))
    db.add(user)
    db.commit()
    db.add(
        LabSession(
            user_id=user.id,
            host_id=seeded_host.id,
            status=SessionStatus.ACTIVE,
        )
    )
    db.commit()

    selected = SingleApprovedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is None


def test_selector_returns_none_for_a_different_tier(db: Session, seeded_host: Host) -> None:
    selected = SingleApprovedHostSelector().select(db, lab_tier="pro")
    assert selected is None
    assert seeded_host.status == HostStatus.APPROVED


def test_selector_returns_none_when_only_pending_host_exists(
    db: Session, pending_host: Host
) -> None:
    selected = SingleApprovedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is None
    assert pending_host.status == HostStatus.PENDING


def test_selector_picks_approved_host_not_pending(
    db: Session, seeded_host: Host, pending_host: Host
) -> None:
    selected = SingleApprovedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == seeded_host.id
    assert selected.id != pending_host.id


def test_selector_skips_approved_host_that_never_pinged(db: Session, seeded_host: Host) -> None:
    seeded_host.last_seen_at = None
    db.commit()

    selected = SingleApprovedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is None


def test_selector_skips_stale_approved_host(db: Session, seeded_host: Host) -> None:
    seeded_host.last_seen_at = datetime.now(timezone.utc) - timedelta(seconds=60)
    db.commit()

    selected = SingleApprovedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is None


def test_selector_picks_host_after_fresh_heartbeat(db: Session, seeded_host: Host) -> None:
    seeded_host.last_seen_at = datetime.now(timezone.utc) - timedelta(seconds=60)
    db.commit()
    assert SingleApprovedHostSelector().select(db, lab_tier=LabTier.STANDARD) is None

    seeded_host.last_seen_at = datetime.now(timezone.utc) - timedelta(seconds=1)
    db.commit()
    selected = SingleApprovedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == seeded_host.id


def test_selector_skips_pending_host_even_with_fresh_heartbeat(
    db: Session, pending_host: Host
) -> None:
    pending_host.last_seen_at = datetime.now(timezone.utc)
    db.commit()

    selected = SingleApprovedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is None
    assert pending_host.status == HostStatus.PENDING
