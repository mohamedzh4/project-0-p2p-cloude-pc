"""Per-host credentials (issue 2): one host cannot act as another."""

from __future__ import annotations

import json

import httpx
from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from matchmaker.host_credentials import (
    HOST_SECRET_HEADER,
    auth_hash,
    command_token,
)
from matchmaker.models import Host
from matchmaker.provision import HttpHostProvisioner
from tests.helpers import host_auth_header, inventory_payload

SECRET_A = "a" * 43
SECRET_B = "b" * 43


def _headers(secret: str | None = None) -> dict[str, str]:
    headers = host_auth_header()
    if secret is not None:
        headers[HOST_SECRET_HEADER] = secret
    return headers


def _register(client: TestClient, secret: str | None = SECRET_A) -> str:
    response = client.post("/v1/hosts", json=inventory_payload(), headers=_headers(secret))
    assert response.status_code == 201, response.text
    return response.json()["hostId"]


def test_registration_stores_derived_values_not_the_secret(
    client: TestClient, db: Session
) -> None:
    host_id = _register(client)
    host = db.get(Host, host_id)
    assert host.secret_hash == auth_hash(SECRET_A)
    assert host.command_token == command_token(SECRET_A)
    assert SECRET_A not in (host.secret_hash, host.command_token)
    assert host.secret_hash != host.command_token


def test_host_response_never_carries_credentials(client: TestClient) -> None:
    response = client.post("/v1/hosts", json=inventory_payload(), headers=_headers(SECRET_A))
    text = response.text
    assert SECRET_A not in text
    assert command_token(SECRET_A) not in text
    assert auth_hash(SECRET_A) not in text


def test_bootstrap_token_alone_cannot_touch_a_host_with_a_secret(client: TestClient) -> None:
    host_id = _register(client)
    for method, path, body in (
        ("post", f"/v1/hosts/{host_id}/heartbeat", None),
        ("get", f"/v1/hosts/{host_id}/earnings", None),
        ("post", f"/v1/hosts/{host_id}/mesh/preauth", None),
        ("put", f"/v1/hosts/{host_id}", inventory_payload(agentUrl="http://evil.example:1")),
    ):
        kwargs = {"headers": _headers()}
        if body is not None:
            kwargs["json"] = body
        response = getattr(client, method)(path, **kwargs)
        assert response.status_code == 403, (path, response.text)


def test_another_hosts_secret_is_refused(client: TestClient, db: Session) -> None:
    host_id = _register(client, SECRET_A)
    _register(client, SECRET_B)
    response = client.put(
        f"/v1/hosts/{host_id}",
        json=inventory_payload(agentUrl="http://evil.example:1"),
        headers=_headers(SECRET_B),
    )
    assert response.status_code == 403
    assert db.get(Host, host_id).agent_url is None


def test_own_secret_is_accepted(client: TestClient) -> None:
    host_id = _register(client)
    assert client.post(f"/v1/hosts/{host_id}/heartbeat", headers=_headers(SECRET_A)).status_code == 200
    assert client.get(f"/v1/hosts/{host_id}/earnings", headers=_headers(SECRET_A)).status_code == 200
    updated = client.put(
        f"/v1/hosts/{host_id}",
        json=inventory_payload(displayName="renamed"),
        headers=_headers(SECRET_A),
    )
    assert updated.status_code == 200
    assert updated.json()["displayName"] == "renamed"


def test_secret_cannot_be_replaced_once_set(client: TestClient, db: Session) -> None:
    host_id = _register(client, SECRET_A)
    client.put(f"/v1/hosts/{host_id}", json=inventory_payload(), headers=_headers(SECRET_A))
    db.expire_all()
    assert db.get(Host, host_id).secret_hash == auth_hash(SECRET_A)


def test_legacy_host_without_a_secret_claims_one_on_first_use(
    client: TestClient, db: Session
) -> None:
    host_id = _register(client, None)
    assert db.get(Host, host_id).secret_hash is None
    assert client.post(f"/v1/hosts/{host_id}/heartbeat", headers=_headers()).status_code == 200
    claimed = client.put(f"/v1/hosts/{host_id}", json=inventory_payload(), headers=_headers(SECRET_A))
    assert claimed.status_code == 200
    assert client.post(f"/v1/hosts/{host_id}/heartbeat", headers=_headers()).status_code == 403


def test_put_with_a_new_id_registers_with_the_secret(client: TestClient, db: Session) -> None:
    host_id = "11111111-2222-3333-4444-555555555555"
    response = client.put(f"/v1/hosts/{host_id}", json=inventory_payload(), headers=_headers(SECRET_A))
    assert response.status_code == 201
    assert db.get(Host, host_id).secret_hash == auth_hash(SECRET_A)


def test_malformed_secret_is_rejected(client: TestClient) -> None:
    for bad in ("short", "has space " + "x" * 40, "x" * 300):
        response = client.post("/v1/hosts", json=inventory_payload(), headers=_headers(bad))
        assert response.status_code == 400, bad


def test_secret_without_bootstrap_token_is_unauthenticated(client: TestClient) -> None:
    response = client.post(
        "/v1/hosts", json=inventory_payload(), headers={HOST_SECRET_HEADER: SECRET_A}
    )
    assert response.status_code == 401


def _capture_provisioner(monkeypatch) -> list[httpx.Request]:
    seen: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return httpx.Response(200, json={"connectionUrl": "https://guest.example/desk"})

    real_client = httpx.Client

    def fake_client(*args, **kwargs):
        kwargs["transport"] = httpx.MockTransport(handler)
        return real_client(*args, **kwargs)

    monkeypatch.setattr("matchmaker.provision.httpx.Client", fake_client)
    return seen


def test_provision_and_teardown_use_the_hosts_command_token(monkeypatch) -> None:
    seen = _capture_provisioner(monkeypatch)
    host = Host(
        display_name="h",
        lab_tier="standard",
        shared_logical_cores=2,
        shared_memory_bytes=1,
        share_gpu=False,
        kasm_url="",
        agent_url="http://agent.example:7070",
        status="approved",
        command_token=command_token(SECRET_A),
    )
    provisioner = HttpHostProvisioner(bearer_token="shared-bootstrap-token")
    assert provisioner.provision(host, lab_tier="standard") is not None
    provisioner.teardown(host)
    expected = f"Bearer {command_token(SECRET_A)}"
    assert [r.headers["Authorization"] for r in seen] == [expected, expected]
    assert json.loads(seen[0].content)["labTier"] == "standard"


def test_legacy_host_is_commanded_with_the_bootstrap_token(monkeypatch) -> None:
    seen = _capture_provisioner(monkeypatch)
    host = Host(
        display_name="h",
        lab_tier="standard",
        shared_logical_cores=2,
        shared_memory_bytes=1,
        share_gpu=False,
        kasm_url="",
        agent_url="http://agent.example:7070",
        status="approved",
    )
    HttpHostProvisioner(bearer_token="shared-bootstrap-token").provision(host, lab_tier="standard")
    assert seen[0].headers["Authorization"] == "Bearer shared-bootstrap-token"
