from __future__ import annotations

from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
from pathlib import Path
from uuid import UUID

from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.config import get_settings
from matchmaker.ledger import billed_minutes
from matchmaker.models import Host, LabSession, LedgerEntry, LedgerParty, SessionStatus
from tests.helpers import auth_header, heartbeat, signup


def _age_host(db: Session, host_id: UUID, seconds: int = 60) -> None:
    host = db.get(Host, host_id)
    assert host is not None
    host.last_seen_at = datetime.now(timezone.utc) - timedelta(seconds=seconds)
    db.commit()


def _backdate_start(db: Session, session_id: str, seconds: int) -> None:
    row = db.get(LabSession, UUID(session_id))
    assert row is not None
    row.started_at = datetime.now(timezone.utc) - timedelta(seconds=seconds)
    db.commit()


def _ledger_rows(db: Session, session_id: str) -> list[LedgerEntry]:
    db.expire_all()
    return list(db.scalars(select(LedgerEntry).where(LedgerEntry.session_id == UUID(session_id))).all())


def test_silence_modules_do_not_name_backends_or_the_concrete_selector() -> None:
    root = Path(__file__).resolve().parents[1] / "src/matchmaker"
    for relative in ("routers/sessions.py", "routers/hosts.py", "silence.py"):
        text = (root / relative).read_text(encoding="utf-8")
        assert "SingleApprovedHostSelector" not in text
        assert "RankedHostSelector" not in text
        assert "Firecracker" not in text
        assert "QEMU" not in text
        assert "WHPX" not in text
        assert "VFIO" not in text


def test_stale_occupied_host_interrupts_and_writes_ledger_once(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    assert created.status_code == 201
    session_id = created.json()["sessionId"]
    _backdate_start(db, session_id, 90)
    _age_host(db, seeded_host.id)

    history = client.get("/v1/sessions", headers=auth_header(token))
    assert history.status_code == 200
    body = history.json()
    assert len(body["sessions"]) == 1
    item = body["sessions"][0]
    assert item["sessionId"] == session_id
    assert item["status"] == "interrupted"
    assert item["endedAt"] is not None
    rate = get_settings().credits_per_minute
    rows = _ledger_rows(db, session_id)
    assert len(rows) == 2
    by_party = {row.party: row for row in rows}
    expected = billed_minutes(by_party[LedgerParty.USER].duration_seconds) * rate
    assert item["creditsDebited"] == expected
    assert body["creditsDebited"] == expected
    assert by_party[LedgerParty.USER].amount_credits == expected
    assert by_party[LedgerParty.HOST].amount_credits == expected
    assert by_party[LedgerParty.HOST].host_id == seeded_host.id

    again = client.get("/v1/sessions", headers=auth_header(token))
    assert again.status_code == 200
    assert again.json()["sessions"][0]["status"] == "interrupted"
    assert len(_ledger_rows(db, session_id)) == 2

    blocked = client.post("/v1/sessions", headers=auth_header(token))
    assert blocked.status_code == 409


def test_second_user_can_use_host_after_silence_if_it_heartbeats_again(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    owner = signup(client, "owner@example.com")["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(owner))
    assert created.status_code == 201
    _age_host(db, seeded_host.id)

    history = client.get("/v1/sessions", headers=auth_header(owner))
    assert history.json()["sessions"][0]["status"] == "interrupted"

    pinged = heartbeat(client, str(seeded_host.id))
    assert pinged.status_code == 200

    other = signup(client, "other@example.com")["accessToken"]
    booked = client.post("/v1/sessions", headers=auth_header(other))
    assert booked.status_code == 201
    assert booked.json()["hostId"] == str(seeded_host.id)


def test_rematch_books_a_second_fresh_host(
    client: TestClient,
    make_approved_host: Callable[..., Host],
    db: Session,
    fake_provisioner,
) -> None:
    weaker = make_approved_host(display_name="weaker", shared_logical_cores=2)
    stronger = make_approved_host(display_name="stronger", shared_logical_cores=8)
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token), json={"labTier": "standard"})
    assert created.status_code == 201
    original_id = created.json()["sessionId"]
    assert created.json()["hostId"] == str(stronger.id)
    _backdate_start(db, original_id, 60)
    _age_host(db, stronger.id)

    rematched = client.post("/v1/sessions", headers=auth_header(token), json={"labTier": "standard"})
    assert rematched.status_code == 200
    assert rematched.json()["hostId"] == str(weaker.id)
    assert rematched.json()["sessionId"] != original_id

    db.expire_all()
    old = db.get(LabSession, UUID(original_id))
    new = db.get(LabSession, UUID(rematched.json()["sessionId"]))
    assert old is not None and old.status == SessionStatus.INTERRUPTED
    assert new is not None and new.status == SessionStatus.ACTIVE
    assert new.host_id == weaker.id
    assert len(_ledger_rows(db, original_id)) == 2
    assert fake_provisioner.calls[-1] == (weaker.id, "standard")

    history = client.get("/v1/sessions", headers=auth_header(token))
    statuses = {item["sessionId"]: item["status"] for item in history.json()["sessions"]}
    assert statuses[original_id] == "interrupted"
    assert statuses[rematched.json()["sessionId"]] == "active"


def test_rematch_is_none_when_no_other_host_is_free(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    session_id = created.json()["sessionId"]
    _age_host(db, seeded_host.id)

    missing = client.post("/v1/sessions", headers=auth_header(token))
    assert missing.status_code == 409
    assert missing.json()["detail"] == "Host unavailable"

    db.expire_all()
    row = db.get(LabSession, UUID(session_id))
    assert row is not None
    assert row.status == SessionStatus.INTERRUPTED
    assert db.scalars(select(LabSession).where(LabSession.status == SessionStatus.ACTIVE)).first() is None


def test_ending_an_interrupted_session_does_not_double_bill_or_rename_status(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    session_id = created.json()["sessionId"]
    _backdate_start(db, session_id, 60)
    _age_host(db, seeded_host.id)
    client.get("/v1/sessions", headers=auth_header(token))

    ended = client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(token))
    assert ended.status_code == 200
    assert ended.json()["status"] == "interrupted"
    assert len(_ledger_rows(db, session_id)) == 2


def test_live_heartbeat_does_not_interrupt_its_own_session(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    session_id = created.json()["sessionId"]

    pinged = heartbeat(client, str(seeded_host.id))
    assert pinged.status_code == 200

    db.expire_all()
    row = db.get(LabSession, UUID(session_id))
    assert row is not None
    assert row.status == SessionStatus.ACTIVE
    other = signup(client, "other@example.com")["accessToken"]
    blocked = client.post("/v1/sessions", headers=auth_header(other))
    assert blocked.status_code == 409


def test_heartbeat_from_a_live_host_reclaims_a_stale_occupied_peer(
    client: TestClient,
    make_approved_host: Callable[..., Host],
    db: Session,
) -> None:
    live = make_approved_host(display_name="live", shared_logical_cores=2)
    occupied = make_approved_host(display_name="occupied", shared_logical_cores=8)
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    assert created.json()["hostId"] == str(occupied.id)
    session_id = created.json()["sessionId"]
    _age_host(db, occupied.id)

    pinged = heartbeat(client, str(live.id))
    assert pinged.status_code == 200

    db.expire_all()
    old = db.get(LabSession, UUID(session_id))
    assert old is not None
    assert old.status == SessionStatus.INTERRUPTED
    active = db.scalars(select(LabSession).where(LabSession.status == SessionStatus.ACTIVE)).first()
    assert active is not None
    assert active.host_id == live.id
    assert active.user_id == old.user_id


def test_concurrent_reclaim_does_not_double_bill(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    session_id = created.json()["sessionId"]
    _backdate_start(db, session_id, 60)
    _age_host(db, seeded_host.id)

    def history() -> int:
        return client.get("/v1/sessions", headers=auth_header(token)).status_code

    with ThreadPoolExecutor(max_workers=2) as pool:
        codes = [future.result() for future in [pool.submit(history), pool.submit(history)]]

    assert sorted(codes) == [200, 200]
    assert len(_ledger_rows(db, session_id)) == 2
