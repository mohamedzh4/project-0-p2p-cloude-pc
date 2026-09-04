from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta, timezone

import pytest
from fastapi.testclient import TestClient
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from matchmaker.auth import hash_password
from matchmaker.models import Host, LabSession, SessionStatus, User
from tests.helpers import KASM_TOKEN, KASM_URL, PROVISION_URL, auth_header, heartbeat, signup


def test_request_session_requires_auth(client: TestClient, seeded_host: Host) -> None:
    response = client.post("/v1/sessions", json={"labTier": "standard"})
    assert response.status_code == 401
    assert seeded_host.id


def test_request_session_creates_and_returns_kasm_connection(
    client: TestClient, seeded_host: Host
) -> None:
    token = signup(client)["accessToken"]
    response = client.post(
        "/v1/sessions",
        headers=auth_header(token),
        json={"labTier": "standard"},
    )
    assert response.status_code == 201
    data = response.json()
    assert data["hostId"] == str(seeded_host.id)
    assert data["labTier"] == "standard"
    assert data["sessionId"]
    assert data["startedAt"]
    assert data["connection"]["kasmUrl"] == KASM_URL
    assert data["connection"]["kasmToken"] == KASM_TOKEN
    assert "intent" not in data


def test_same_user_second_request_returns_existing_session(
    client: TestClient, seeded_host: Host
) -> None:
    token = signup(client)["accessToken"]
    first = client.post("/v1/sessions", headers=auth_header(token))
    second = client.post("/v1/sessions", headers=auth_header(token))
    assert first.status_code == 201
    assert second.status_code == 200
    assert first.json()["sessionId"] == second.json()["sessionId"]
    assert seeded_host.id


def test_session_router_does_not_name_backends_or_the_concrete_selector() -> None:
    from pathlib import Path

    source = Path(__file__).resolve().parents[1] / "src/matchmaker/routers/sessions.py"
    text = source.read_text(encoding="utf-8")
    assert "SingleApprovedHostSelector" not in text
    assert "Firecracker" not in text
    assert "QEMU" not in text
    assert "WHPX" not in text
    assert "VFIO" not in text


def test_second_user_is_rejected_while_host_is_occupied(
    client: TestClient, seeded_host: Host
) -> None:
    first_token = signup(client, "one@example.com")["accessToken"]
    second_token = signup(client, "two@example.com")["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(first_token))
    blocked = client.post("/v1/sessions", headers=auth_header(second_token))
    assert created.status_code == 201
    assert blocked.status_code == 409
    assert seeded_host.id


def test_wrong_lab_tier_is_unavailable(client: TestClient, seeded_host: Host) -> None:
    """FR2: mapped Pro bookings 409 when inventory is Standard-only. No fake GPU session."""
    token = signup(client)["accessToken"]
    response = client.post(
        "/v1/sessions",
        headers=auth_header(token),
        json={"labTier": "pro"},
    )
    assert response.status_code == 409
    assert response.json()["detail"] == "Host unavailable"
    assert seeded_host.id


def test_request_without_a_host_is_unavailable(client: TestClient) -> None:
    token = signup(client)["accessToken"]
    response = client.post("/v1/sessions", headers=auth_header(token))
    assert response.status_code == 409


def test_ending_a_session_frees_the_host(client: TestClient, seeded_host: Host) -> None:
    first_token = signup(client, "one@example.com")["accessToken"]
    second_token = signup(client, "two@example.com")["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(first_token))
    session_id = created.json()["sessionId"]

    ended = client.post(
        f"/v1/sessions/{session_id}/end",
        headers=auth_header(first_token),
    )
    assert ended.status_code == 200
    assert ended.json()["status"] == "ended"
    assert ended.json()["endedAt"]

    taken = client.post("/v1/sessions", headers=auth_header(second_token))
    assert taken.status_code == 201
    assert taken.json()["hostId"] == str(seeded_host.id)


def test_cannot_end_someone_elses_session(client: TestClient, seeded_host: Host) -> None:
    owner = signup(client, "owner@example.com")["accessToken"]
    other = signup(client, "other@example.com")["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(owner))
    session_id = created.json()["sessionId"]

    response = client.post(
        f"/v1/sessions/{session_id}/end",
        headers=auth_header(other),
    )
    assert response.status_code == 404
    assert seeded_host.id


def test_unique_index_rejects_second_active_session_on_same_host(
    db: Session, seeded_host: Host
) -> None:
    user_one = User(email="one@example.com", password_hash=hash_password("password123"))
    user_two = User(email="two@example.com", password_hash=hash_password("password123"))
    db.add_all([user_one, user_two])
    db.commit()

    first = LabSession(
        user_id=user_one.id,
        host_id=seeded_host.id,
        status=SessionStatus.ACTIVE,
    )
    db.add(first)
    db.commit()

    second = LabSession(
        user_id=user_two.id,
        host_id=seeded_host.id,
        status=SessionStatus.ACTIVE,
    )
    db.add(second)
    with pytest.raises(IntegrityError):
        db.commit()


def test_concurrent_requests_only_one_gets_the_host(
    client: TestClient, seeded_host: Host
) -> None:
    token_a = signup(client, "a@example.com")["accessToken"]
    token_b = signup(client, "b@example.com")["accessToken"]

    def take(token: str):
        return client.post("/v1/sessions", headers=auth_header(token))

    with ThreadPoolExecutor(max_workers=2) as pool:
        results = [future.result() for future in [pool.submit(take, token_a), pool.submit(take, token_b)]]

    codes = sorted(response.status_code for response in results)
    assert codes == [201, 409], [response.json() for response in results]
    winners = [response.json() for response in results if response.status_code == 201]
    assert winners[0]["hostId"] == str(seeded_host.id)


def test_new_session_kasm_url_comes_from_provision_result(
    client: TestClient, seeded_host: Host, fake_provisioner
) -> None:
    token = signup(client)["accessToken"]
    response = client.post(
        "/v1/sessions",
        headers=auth_header(token),
        json={"labTier": "standard"},
    )
    assert response.status_code == 201
    data = response.json()
    assert data["connection"]["kasmUrl"] == PROVISION_URL
    assert data["connection"]["kasmUrl"] != KASM_URL
    assert data["connection"]["kasmToken"] == KASM_TOKEN
    assert data["hostId"] == str(seeded_host.id)
    assert fake_provisioner.calls == [(seeded_host.id, "standard")]


def test_existing_session_does_not_provision_again(
    client: TestClient, seeded_host: Host, fake_provisioner
) -> None:
    token = signup(client)["accessToken"]
    first = client.post("/v1/sessions", headers=auth_header(token))
    second = client.post("/v1/sessions", headers=auth_header(token))
    assert first.status_code == 201
    assert second.status_code == 200
    assert first.json()["sessionId"] == second.json()["sessionId"]
    assert second.json()["connection"]["kasmUrl"] == PROVISION_URL
    assert len(fake_provisioner.calls) == 1
    assert seeded_host.id


def test_double_book_still_409_when_provision_succeeds(
    client: TestClient, seeded_host: Host, fake_provisioner
) -> None:
    first_token = signup(client, "one@example.com")["accessToken"]
    second_token = signup(client, "two@example.com")["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(first_token))
    blocked = client.post("/v1/sessions", headers=auth_header(second_token))
    assert created.status_code == 201
    assert created.json()["connection"]["kasmUrl"] == PROVISION_URL
    assert blocked.status_code == 409
    assert len(fake_provisioner.calls) == 1
    assert seeded_host.id


def test_concurrent_requests_only_one_gets_the_host_with_provision(
    client: TestClient, seeded_host: Host, fake_provisioner
) -> None:
    token_a = signup(client, "a@example.com")["accessToken"]
    token_b = signup(client, "b@example.com")["accessToken"]

    def take(token: str):
        return client.post("/v1/sessions", headers=auth_header(token))

    with ThreadPoolExecutor(max_workers=2) as pool:
        results = [future.result() for future in [pool.submit(take, token_a), pool.submit(take, token_b)]]

    codes = sorted(response.status_code for response in results)
    assert codes == [201, 409], [response.json() for response in results]
    winners = [response.json() for response in results if response.status_code == 201]
    assert winners[0]["hostId"] == str(seeded_host.id)
    assert winners[0]["connection"]["kasmUrl"] == PROVISION_URL
    assert 1 <= len(fake_provisioner.calls) <= 2


def test_stale_approved_host_is_unavailable(client: TestClient, seeded_host: Host, db: Session) -> None:
    seeded_host.last_seen_at = datetime.now(timezone.utc) - timedelta(seconds=60)
    db.commit()

    token = signup(client)["accessToken"]
    response = client.post("/v1/sessions", headers=auth_header(token))
    assert response.status_code == 409


def test_fresh_heartbeat_allows_booking(client: TestClient, seeded_host: Host, db: Session) -> None:
    seeded_host.last_seen_at = datetime.now(timezone.utc) - timedelta(seconds=60)
    db.commit()

    token = signup(client)["accessToken"]
    stale = client.post("/v1/sessions", headers=auth_header(token))
    assert stale.status_code == 409

    pinged = heartbeat(client, str(seeded_host.id))
    assert pinged.status_code == 200
    assert pinged.json()["lastSeenAt"] is not None

    booked = client.post("/v1/sessions", headers=auth_header(token))
    assert booked.status_code == 201
    assert booked.json()["hostId"] == str(seeded_host.id)


def test_never_seen_approved_host_is_unavailable(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    seeded_host.last_seen_at = None
    db.commit()

    token = signup(client)["accessToken"]
    response = client.post("/v1/sessions", headers=auth_header(token))
    assert response.status_code == 409
