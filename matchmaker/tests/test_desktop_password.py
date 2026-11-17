"""Per-session desktop password.

A guest's desktop (KasmVNC) is reachable by other machines on the mesh:
every Host Agent may reach every guest on the streamer port. So the guest
asks for a password that only it and the gateway know. A renter never sees
it (the gateway adds it), and a host never receives it.
"""

from __future__ import annotations

import re
from collections.abc import Generator
from uuid import uuid4

import pytest
from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from matchmaker.config import get_settings
from matchmaker.guest_bootstrap import (
    DESKTOP_USER,
    desktop_credentials,
    new_desktop_password,
)
from matchmaker.models import Host
from tests.helpers import FakeHostProvisioner, auth_header, signup

GATEWAY_ORIGIN = "https://gateway.test.example"
GATEWAY_RESOLVE = "test-gateway-resolve-token"


@pytest.fixture
def gateway(monkeypatch) -> Generator[None, None, None]:
    monkeypatch.setenv("GATEWAY_PUBLIC_ORIGIN", GATEWAY_ORIGIN)
    monkeypatch.setenv("GATEWAY_RESOLVE_TOKEN", GATEWAY_RESOLVE)
    get_settings.cache_clear()
    yield
    monkeypatch.delenv("GATEWAY_PUBLIC_ORIGIN", raising=False)
    monkeypatch.delenv("GATEWAY_RESOLVE_TOKEN", raising=False)
    get_settings.cache_clear()


def _book(client: TestClient, email: str):
    token = signup(client, email=email)["accessToken"]
    booked = client.post("/v1/sessions", headers=auth_header(token))
    assert booked.status_code == 201, booked.text
    return token, booked


def _redeem(client: TestClient, provisioner: FakeHostProvisioner) -> dict:
    hint = provisioner.mesh_calls[-1]
    redeemed = client.post(
        "/v1/guest/bootstrap", headers={"Authorization": f"Bearer {hint.bootstrap_token}"}
    )
    assert redeemed.status_code == 200, redeemed.text
    return redeemed.json()


def _resolve(client: TestClient, booked) -> dict:
    grant = booked.json()["connection"]["kasmUrl"].split("/s/")[1].rstrip("/")
    resolved = client.post(
        "/v1/gateway/resolve",
        json={"token": grant},
        headers={"Authorization": f"Bearer {GATEWAY_RESOLVE}"},
    )
    assert resolved.status_code == 200, resolved.text
    return resolved.json()


def test_passwords_are_long_random_and_shell_safe() -> None:
    first, second = new_desktop_password(), new_desktop_password()
    assert first != second
    assert re.fullmatch(r"[A-Za-z0-9]{32}", first)


def test_guest_and_gateway_get_the_same_password_and_nobody_else_does(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    gateway: None,
) -> None:
    token, booked = _book(client, "ada@example.com")
    guest = _redeem(client, fake_provisioner)
    password = guest["desktopPassword"]
    assert guest["desktopUser"] == DESKTOP_USER == "ubuntu"
    assert re.fullmatch(r"[A-Za-z0-9]{32}", password)

    resolved = _resolve(client, booked)
    assert resolved["upstreamUser"] == "ubuntu"
    assert resolved["upstreamPassword"] == password

    # The renter: not in the booking reply, the session, or the history.
    session_id = booked.json()["sessionId"]
    for reply in (
        booked,
        client.get(f"/v1/sessions/{session_id}", headers=auth_header(token)),
        client.get("/v1/sessions/current", headers=auth_header(token)),
        client.get("/v1/sessions", headers=auth_header(token)),
    ):
        assert password not in reply.text
        assert "desktopPassword" not in reply.text
        assert "upstreamPassword" not in reply.text

    # The host: the provision hint carries the bootstrap token, not the password.
    hint = fake_provisioner.mesh_calls[-1]
    assert password not in repr(hint)
    assert password not in repr(fake_provisioner.storage_calls[-1])


def test_each_session_has_its_own_password(
    client: TestClient,
    make_approved_host,
    fake_provisioner: FakeHostProvisioner,
    gateway: None,
) -> None:
    make_approved_host(display_name="one")
    make_approved_host(display_name="two")
    _, first = _book(client, "a@example.com")
    first_password = _redeem(client, fake_provisioner)["desktopPassword"]
    _, second = _book(client, "b@example.com")
    second_password = _redeem(client, fake_provisioner)["desktopPassword"]
    assert first_password != second_password
    assert _resolve(client, first)["upstreamPassword"] == first_password
    assert _resolve(client, second)["upstreamPassword"] == second_password


def test_password_is_gone_when_the_session_ends(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    gateway: None,
    db: Session,
) -> None:
    token, booked = _book(client, "ada@example.com")
    session_id = booked.json()["sessionId"]
    assert desktop_credentials(db, session_id) is not None
    client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(token))
    db.expire_all()
    assert desktop_credentials(db, session_id) is None
    grant = booked.json()["connection"]["kasmUrl"].split("/s/")[1].rstrip("/")
    gone = client.post(
        "/v1/gateway/resolve",
        json={"token": grant},
        headers={"Authorization": f"Bearer {GATEWAY_RESOLVE}"},
    )
    assert gone.status_code == 404


def test_resolve_requires_the_gateway_token_to_see_the_password(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    gateway: None,
) -> None:
    token, booked = _book(client, "ada@example.com")
    password = _redeem(client, fake_provisioner)["desktopPassword"]
    grant = booked.json()["connection"]["kasmUrl"].split("/s/")[1].rstrip("/")
    for headers in ({}, auth_header(token), {"Authorization": "Bearer wrong-token-value-000"}):
        denied = client.post("/v1/gateway/resolve", json={"token": grant}, headers=headers)
        assert denied.status_code == 401
        assert password not in denied.text


def test_unknown_session_has_no_desktop_credentials(db: Session) -> None:
    assert desktop_credentials(db, uuid4()) is None
