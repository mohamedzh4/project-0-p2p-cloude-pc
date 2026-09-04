from __future__ import annotations

from sqlalchemy.orm import Session

from matchmaker.auth import hash_password
from matchmaker.matching import SingleApprovedHostSelector
from matchmaker.models import Host, HostStatus, LabSession, LabTier, SessionStatus, User


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
