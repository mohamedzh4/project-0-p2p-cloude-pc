from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone
from uuid import UUID

from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.config import get_settings
from matchmaker.ledger import billed_minutes, credits_for_duration
from matchmaker.models import Host, LabSession, LedgerEntry, LedgerParty
from tests.helpers import auth_header, host_auth_header, signup


def _backdate_start(db: Session, session_id: str, seconds: int) -> None:
    row = db.get(LabSession, UUID(session_id))
    assert row is not None
    row.started_at = datetime.now(timezone.utc) - timedelta(seconds=seconds)
    db.commit()


def test_billed_minutes_rounds_up_and_zero_is_free() -> None:
    assert billed_minutes(0) == 0
    assert billed_minutes(1) == 1
    assert billed_minutes(60) == 1
    assert billed_minutes(61) == 2


def test_credits_for_duration_uses_env_rate() -> None:
    start = datetime(2026, 9, 4, 12, 0, tzinfo=timezone.utc)
    end = start + timedelta(seconds=90)
    seconds, amount = credits_for_duration(start, end, credits_per_minute=3)
    assert seconds == 90
    assert amount == 6


def test_ending_a_session_writes_ledger(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    assert created.status_code == 201
    session_id = created.json()["sessionId"]
    _backdate_start(db, session_id, 90)

    ended = client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(token))
    assert ended.status_code == 200

    rows = db.scalars(select(LedgerEntry).where(LedgerEntry.session_id == UUID(session_id))).all()
    assert len(rows) == 2
    by_party = {row.party: row for row in rows}
    rate = get_settings().credits_per_minute
    expected = billed_minutes(by_party[LedgerParty.USER].duration_seconds) * rate
    assert ended.json()["creditsDebited"] == expected
    assert by_party[LedgerParty.USER].direction == "debit"
    assert by_party[LedgerParty.USER].user_id is not None
    assert by_party[LedgerParty.USER].host_id is None
    assert by_party[LedgerParty.HOST].direction == "credit"
    assert by_party[LedgerParty.HOST].host_id == seeded_host.id
    assert by_party[LedgerParty.HOST].user_id is None
    assert by_party[LedgerParty.USER].amount_credits == expected
    assert by_party[LedgerParty.HOST].amount_credits == expected
    assert by_party[LedgerParty.USER].duration_seconds >= 90
    assert by_party[LedgerParty.USER].rate_credits_per_minute == rate


def test_ending_twice_does_not_double_credit(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    session_id = created.json()["sessionId"]
    _backdate_start(db, session_id, 120)

    first = client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(token))
    second = client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(token))
    assert first.status_code == 200
    assert second.status_code == 200
    assert first.json()["creditsDebited"] == second.json()["creditsDebited"]

    rows = db.scalars(select(LedgerEntry).where(LedgerEntry.session_id == UUID(session_id))).all()
    assert len(rows) == 2
    assert seeded_host.id


def test_concurrent_end_does_not_double_credit(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    session_id = created.json()["sessionId"]
    _backdate_start(db, session_id, 60)

    def end() -> int:
        response = client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(token))
        return response.status_code

    with ThreadPoolExecutor(max_workers=2) as pool:
        codes = [future.result() for future in [pool.submit(end), pool.submit(end)]]

    assert sorted(codes) == [200, 200]
    rows = db.scalars(select(LedgerEntry).where(LedgerEntry.session_id == UUID(session_id))).all()
    assert len(rows) == 2
    assert seeded_host.id


def test_session_history_lists_owner_sessions_and_credits(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    owner = signup(client, "owner@example.com")["accessToken"]
    other = signup(client, "other@example.com")["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(owner))
    session_id = created.json()["sessionId"]
    _backdate_start(db, session_id, 60)
    ended = client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(owner))
    assert ended.status_code == 200

    other_booked = client.post("/v1/sessions", headers=auth_header(other))
    assert other_booked.status_code == 201

    history = client.get("/v1/sessions", headers=auth_header(owner))
    assert history.status_code == 200
    body = history.json()
    assert body["creditsDebited"] == ended.json()["creditsDebited"]
    assert len(body["sessions"]) == 1
    item = body["sessions"][0]
    assert item["sessionId"] == session_id
    assert item["hostId"] == str(seeded_host.id)
    assert item["status"] == "ended"
    assert item["creditsDebited"] == body["creditsDebited"]
    assert item["durationSeconds"] >= 60
    assert item["labTier"] == "standard"

    other_history = client.get("/v1/sessions", headers=auth_header(other))
    assert other_history.status_code == 200
    assert other_history.json()["creditsDebited"] == 0
    assert other_history.json()["sessions"][0]["status"] == "active"
    assert other_history.json()["sessions"][0]["durationSeconds"] is None


def test_session_history_requires_auth(client: TestClient) -> None:
    response = client.get("/v1/sessions")
    assert response.status_code == 401


def test_host_earnings_are_credits_accrued(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    empty = client.get(
        f"/v1/hosts/{seeded_host.id}/earnings",
        headers=host_auth_header(),
    )
    assert empty.status_code == 200
    assert empty.json() == {"hostId": str(seeded_host.id), "creditsAccrued": 0}
    assert "paid" not in empty.json()

    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    session_id = created.json()["sessionId"]
    _backdate_start(db, session_id, 60)
    ended = client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(token))
    assert ended.status_code == 200

    earned = client.get(
        f"/v1/hosts/{seeded_host.id}/earnings",
        headers=host_auth_header(),
    )
    assert earned.status_code == 200
    assert earned.json()["creditsAccrued"] == ended.json()["creditsDebited"]


def test_host_earnings_rejects_user_bearer(
    client: TestClient, seeded_host: Host
) -> None:
    token = signup(client)["accessToken"]
    response = client.get(
        f"/v1/hosts/{seeded_host.id}/earnings",
        headers=auth_header(token),
    )
    assert response.status_code == 401


def test_host_earnings_unknown_host_is_not_found(client: TestClient) -> None:
    response = client.get(
        "/v1/hosts/00000000-0000-4000-8000-000000000099/earnings",
        headers=host_auth_header(),
    )
    assert response.status_code == 404
