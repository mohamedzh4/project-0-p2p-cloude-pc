from __future__ import annotations

from collections.abc import Callable
from datetime import datetime, timedelta, timezone

from sqlalchemy.orm import Session

from matchmaker.auth import hash_password
from matchmaker.deps import get_host_selector
from matchmaker.matching import (
    HEARTBEAT_STALE_AFTER,
    RankedHostSelector,
    heartbeat_freshness_cutoff,
    heartbeat_is_fresh,
)
from matchmaker.models import Host, HostStatus, LabSession, LabTier, SessionStatus, User

GiB = 1024 * 1024 * 1024


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


def test_get_host_selector_wires_ranked_implementation() -> None:
    selector = get_host_selector()
    assert type(selector) is RankedHostSelector


def test_selector_returns_the_approved_host_when_free(db: Session, seeded_host: Host) -> None:
    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
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

    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is None


def test_selector_returns_none_for_a_different_tier(db: Session, seeded_host: Host) -> None:
    selected = RankedHostSelector().select(db, lab_tier=LabTier.PRO)
    assert selected is None
    assert seeded_host.status == HostStatus.APPROVED


def test_selector_returns_none_when_only_pending_host_exists(
    db: Session, pending_host: Host
) -> None:
    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is None
    assert pending_host.status == HostStatus.PENDING


def test_selector_picks_approved_host_not_pending(
    db: Session, seeded_host: Host, pending_host: Host
) -> None:
    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == seeded_host.id
    assert selected.id != pending_host.id


def test_selector_skips_approved_host_that_never_pinged(db: Session, seeded_host: Host) -> None:
    seeded_host.last_seen_at = None
    db.commit()

    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is None


def test_selector_skips_stale_approved_host(db: Session, seeded_host: Host) -> None:
    seeded_host.last_seen_at = datetime.now(timezone.utc) - timedelta(seconds=60)
    db.commit()

    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is None


def test_selector_picks_host_after_fresh_heartbeat(db: Session, seeded_host: Host) -> None:
    seeded_host.last_seen_at = datetime.now(timezone.utc) - timedelta(seconds=60)
    db.commit()
    assert RankedHostSelector().select(db, lab_tier=LabTier.STANDARD) is None

    seeded_host.last_seen_at = datetime.now(timezone.utc) - timedelta(seconds=1)
    db.commit()
    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == seeded_host.id


def test_selector_skips_pending_host_even_with_fresh_heartbeat(
    db: Session, pending_host: Host
) -> None:
    pending_host.last_seen_at = datetime.now(timezone.utc)
    db.commit()

    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is None
    assert pending_host.status == HostStatus.PENDING


def test_two_free_approved_hosts_pick_higher_declared_cores(
    db: Session, make_approved_host: Callable[..., Host]
) -> None:
    weaker = make_approved_host(display_name="weaker", shared_logical_cores=2, shared_memory_bytes=16 * GiB)
    stronger = make_approved_host(display_name="stronger", shared_logical_cores=8, shared_memory_bytes=4 * GiB)

    first = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    second = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert first is not None
    assert first.id == stronger.id
    assert first.id != weaker.id
    assert second is not None
    assert second.id == first.id


def test_equal_cores_pick_higher_declared_memory(
    db: Session, make_approved_host: Callable[..., Host]
) -> None:
    less_ram = make_approved_host(display_name="less-ram", shared_logical_cores=4, shared_memory_bytes=4 * GiB)
    more_ram = make_approved_host(display_name="more-ram", shared_logical_cores=4, shared_memory_bytes=16 * GiB)

    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == more_ram.id
    assert selected.id != less_ram.id


def test_equal_caps_tie_break_is_deterministic_by_host_id(
    db: Session, make_approved_host: Callable[..., Host]
) -> None:
    left = make_approved_host(display_name="left", shared_logical_cores=4, shared_memory_bytes=8 * GiB)
    right = make_approved_host(display_name="right", shared_logical_cores=4, shared_memory_bytes=8 * GiB)
    expected_id = min(left.id, right.id)

    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == expected_id
    again = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert again is not None
    assert again.id == expected_id


def test_occupied_winner_is_skipped_for_next_ranked_host(
    db: Session, make_approved_host: Callable[..., Host]
) -> None:
    runner_up = make_approved_host(display_name="runner-up", shared_logical_cores=2, shared_memory_bytes=4 * GiB)
    winner = make_approved_host(display_name="winner", shared_logical_cores=8, shared_memory_bytes=16 * GiB)
    user = User(email="guest@example.com", password_hash=hash_password("password123"))
    db.add(user)
    db.commit()
    db.add(LabSession(user_id=user.id, host_id=winner.id, status=SessionStatus.ACTIVE))
    db.commit()

    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == runner_up.id


def test_stale_winner_is_skipped_for_fresh_weaker_host(
    db: Session, make_approved_host: Callable[..., Host]
) -> None:
    fresh = make_approved_host(display_name="fresh-weak", shared_logical_cores=2)
    make_approved_host(
        display_name="stale-strong",
        shared_logical_cores=16,
        last_seen_at=datetime.now(timezone.utc) - timedelta(seconds=60),
    )

    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == fresh.id


def test_pending_higher_caps_are_excluded(
    db: Session, make_approved_host: Callable[..., Host]
) -> None:
    approved = make_approved_host(display_name="approved-small", shared_logical_cores=2)
    make_approved_host(
        display_name="pending-huge",
        shared_logical_cores=64,
        status=HostStatus.PENDING,
    )

    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == approved.id


def test_wrong_tier_host_is_excluded_even_with_more_caps(
    db: Session, make_approved_host: Callable[..., Host]
) -> None:
    standard = make_approved_host(display_name="standard", shared_logical_cores=2)
    make_approved_host(
        display_name="pro-gpu",
        lab_tier=LabTier.PRO,
        shared_logical_cores=32,
        share_gpu=True,
        shared_gpu_id="GPU-0",
    )

    selected = RankedHostSelector().select(db, lab_tier=LabTier.STANDARD)
    assert selected is not None
    assert selected.id == standard.id
    assert RankedHostSelector().select(db, lab_tier=LabTier.PRO) is not None


def test_pro_requires_dedicated_gpu_id(db: Session, make_approved_host: Callable[..., Host]) -> None:
    make_approved_host(
        display_name="pro-no-gpu",
        lab_tier=LabTier.PRO,
        shared_logical_cores=16,
        share_gpu=False,
        shared_gpu_id=None,
    )
    assert RankedHostSelector().select(db, lab_tier=LabTier.PRO) is None

    with_gpu = make_approved_host(
        display_name="pro-gpu",
        lab_tier=LabTier.PRO,
        shared_logical_cores=8,
        share_gpu=True,
        shared_gpu_id="GPU-1",
    )
    selected = RankedHostSelector().select(db, lab_tier=LabTier.PRO)
    assert selected is not None
    assert selected.id == with_gpu.id
    assert selected.shared_gpu_id == "GPU-1"


def test_pro_does_not_treat_gpu_as_a_concurrent_slice(
    db: Session, make_approved_host: Callable[..., Host]
) -> None:
    gpu_host = make_approved_host(
        display_name="pro-gpu",
        lab_tier=LabTier.PRO,
        share_gpu=True,
        shared_gpu_id="GPU-EXCLUSIVE",
    )
    user = User(email="guest@example.com", password_hash=hash_password("password123"))
    db.add(user)
    db.commit()
    db.add(LabSession(user_id=user.id, host_id=gpu_host.id, status=SessionStatus.ACTIVE))
    db.commit()

    assert RankedHostSelector().select(db, lab_tier=LabTier.PRO) is None


def test_pro_blank_gpu_id_is_not_selected(db: Session, make_approved_host: Callable[..., Host]) -> None:
    make_approved_host(
        display_name="pro-blank",
        lab_tier=LabTier.PRO,
        share_gpu=True,
        shared_gpu_id="",
    )
    assert RankedHostSelector().select(db, lab_tier=LabTier.PRO) is None
