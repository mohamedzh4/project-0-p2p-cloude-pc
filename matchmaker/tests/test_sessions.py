from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor

import pytest
from fastapi.testclient import TestClient
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from matchmaker.auth import hash_password
from matchmaker.models import Host, LabSession, SessionStatus, User
from tests.helpers import KASM_TOKEN, KASM_URL, auth_header, signup


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
    token = signup(client)["accessToken"]
    response = client.post(
        "/v1/sessions",
        headers=auth_header(token),
        json={"labTier": "pro"},
    )
    assert response.status_code == 409
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
