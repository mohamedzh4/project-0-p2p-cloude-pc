"""Hosts behind a home router fetch their commands themselves (pull mode)."""

from __future__ import annotations

import json
import threading
import time
from datetime import datetime, timedelta, timezone
from uuid import uuid4

from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker import host_commands
from matchmaker.db import get_session_factory
from matchmaker.host_commands import (
    KIND_PROVISION,
    KIND_TEARDOWN,
    RoutingHostProvisioner,
)
from matchmaker.host_credentials import HOST_SECRET_HEADER, command_token
from matchmaker.mesh import GuestMeshHint
from matchmaker.models import Host, HostCommand
from matchmaker.provision import HttpHostProvisioner
from tests.helpers import host_auth_header, inventory_payload

SECRET_A = "a" * 43
SECRET_B = "b" * 43
LIVE_URL = "https://dcl-guest.test.example:8443/"


def _headers(secret: str | None) -> dict[str, str]:
    headers = host_auth_header()
    if secret is not None:
        headers[HOST_SECRET_HEADER] = secret
    return headers


def _register(client: TestClient, secret: str | None = SECRET_A, **overrides) -> str:
    response = client.post(
        "/v1/hosts", json=inventory_payload(**overrides), headers=_headers(secret)
    )
    assert response.status_code == 201, response.text
    return response.json()["hostId"]


def _poll(client: TestClient, host_id: str, secret: str | None = SECRET_A):
    return client.post(f"/v1/hosts/{host_id}/commands/poll", headers=_headers(secret))


def _provisioner(**timeouts) -> RoutingHostProvisioner:
    http = HttpHostProvisioner(bearer_token="shared-bootstrap-token")
    return RoutingHostProvisioner(
        http, get_session_factory(), poll_interval_seconds=0.02, **timeouts
    )


def _hint() -> GuestMeshHint:
    return GuestMeshHint(
        login_server="https://mesh.test.example",
        hostname="dcl-guest-x",
        exit_node=None,
        acl_tags=("tag:guest",),
        bootstrap_origin="https://mm.test.example",
        bootstrap_token="guest-bootstrap-token-value",
    )


def test_poll_is_empty_and_marks_the_host_as_pull_mode(client: TestClient, db: Session) -> None:
    host_id = _register(client)
    assert db.get(Host, host_id).pull_mode is False
    assert _poll(client, host_id).status_code == 204
    db.expire_all()
    assert db.get(Host, host_id).pull_mode is True


def test_only_the_host_itself_can_read_its_commands(client: TestClient, db: Session) -> None:
    host_id = _register(client, SECRET_A)
    _register(client, SECRET_B)
    host_commands.enqueue(db, db.get(Host, host_id).id, KIND_PROVISION, {"labTier": "standard"})

    assert _poll(client, host_id, None).status_code == 403  # shared token only
    assert _poll(client, host_id, SECRET_B).status_code == 403  # another host
    assert client.post(f"/v1/hosts/{host_id}/commands/poll").status_code == 401
    assert _poll(client, str(uuid4())).status_code == 404

    mine = _poll(client, host_id, SECRET_A)
    assert mine.status_code == 200
    assert mine.json()["body"] == {"labTier": "standard"}


def test_a_host_without_its_own_secret_cannot_poll(client: TestClient) -> None:
    legacy = _register(client, None)
    response = _poll(client, legacy, None)
    assert response.status_code == 403
    assert "secret" in response.json()["detail"]


def test_command_carries_the_command_token_and_is_delivered_once(
    client: TestClient, db: Session
) -> None:
    host_id = _register(client)
    command_id = host_commands.enqueue(
        db, db.get(Host, host_id).id, KIND_PROVISION, {"guestMesh": {"bootstrapToken": "tok"}}
    )
    first = _poll(client, host_id)
    body = first.json()
    assert body["commandId"] == str(command_id)
    assert body["kind"] == "provision"
    assert body["commandToken"] == command_token(SECRET_A)
    assert SECRET_A not in first.text
    assert _poll(client, host_id).status_code == 204

    # The payload (it holds a guest bootstrap token) is erased once taken.
    db.expire_all()
    row = db.get(HostCommand, command_id)
    assert row.status == "taken"
    assert row.payload is None


def test_commands_come_oldest_first(client: TestClient, db: Session) -> None:
    host_id = _register(client)
    host = db.get(Host, host_id).id
    host_commands.enqueue(db, host, KIND_TEARDOWN)
    host_commands.enqueue(db, host, KIND_PROVISION, {"labTier": "standard"})
    assert [_poll(client, host_id).json()["kind"] for _ in range(2)] == ["teardown", "provision"]


def test_result_is_accepted_once_from_the_right_host(client: TestClient, db: Session) -> None:
    host_id = _register(client, SECRET_A)
    other_id = _register(client, SECRET_B)
    command_id = host_commands.enqueue(db, db.get(Host, host_id).id, KIND_PROVISION, {})
    path = f"/v1/hosts/{host_id}/commands/{command_id}/result"
    result = {"status": 200, "body": json.dumps({"connectionUrl": LIVE_URL})}

    # Not taken yet, wrong host, wrong secret.
    assert client.post(path, json=result, headers=_headers(SECRET_A)).status_code == 404
    _poll(client, host_id)
    assert client.post(path, json=result, headers=_headers(SECRET_B)).status_code == 403
    wrong_host = f"/v1/hosts/{other_id}/commands/{command_id}/result"
    assert client.post(wrong_host, json=result, headers=_headers(SECRET_B)).status_code == 404

    assert client.post(path, json=result, headers=_headers(SECRET_A)).status_code == 204
    assert client.post(path, json=result, headers=_headers(SECRET_A)).status_code == 404
    db.expire_all()
    row = db.get(HostCommand, command_id)
    assert (row.status, row.result_status) == ("done", 200)


def test_stale_provision_commands_are_never_delivered(client: TestClient, db: Session) -> None:
    host_id = _register(client)
    command_id = host_commands.enqueue(db, db.get(Host, host_id).id, KIND_PROVISION, {"x": 1})
    row = db.get(HostCommand, command_id)
    row.created_at = datetime.now(timezone.utc) - timedelta(minutes=5)
    db.commit()
    assert _poll(client, host_id).status_code == 204
    db.expire_all()
    row = db.get(HostCommand, command_id)
    assert row.status == "expired"
    assert row.payload is None


def _agent(client: TestClient, host_id: str, reply: dict | None, seen: list, status: int = 200):
    """A thread that plays the Host Agent: poll, then answer."""

    def run() -> None:
        deadline = time.time() + 5
        while time.time() < deadline:
            polled = _poll(client, host_id)
            if polled.status_code == 200:
                command = polled.json()
                seen.append(command)
                if reply is not None or status != 200:
                    client.post(
                        f"/v1/hosts/{host_id}/commands/{command['commandId']}/result",
                        json={"status": status, "body": json.dumps(reply or {})},
                        headers=_headers(SECRET_A),
                    )
                return
            time.sleep(0.02)

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    return thread


def test_provision_reaches_a_pull_mode_host_and_returns_its_desktop(
    client: TestClient, db: Session
) -> None:
    host_id = _register(client)
    _poll(client, host_id)  # the agent has announced itself
    db.expire_all()
    host = db.get(Host, host_id)
    seen: list = []
    agent = _agent(client, host_id, {"connectionUrl": LIVE_URL, "streamer": "kasm"}, seen)

    result = _provisioner().provision(host, lab_tier="standard", guest_mesh=_hint())
    agent.join(timeout=5)

    assert result is not None and not result.failed
    assert result.connection_url == LIVE_URL
    command = seen[0]
    assert command["kind"] == "provision"
    assert command["commandToken"] == command_token(SECRET_A)
    assert command["body"]["labTier"] == "standard"
    assert command["body"]["guestMesh"]["bootstrapToken"] == "guest-bootstrap-token-value"


def test_agent_error_is_a_failed_booking(client: TestClient, db: Session) -> None:
    host_id = _register(client)
    _poll(client, host_id)
    db.expire_all()
    agent = _agent(client, host_id, None, [], status=400)
    result = _provisioner().provision(db.get(Host, host_id), lab_tier="standard")
    agent.join(timeout=5)
    assert result is not None and result.failed


def test_an_agent_that_never_polls_fails_the_booking_and_gets_no_late_command(
    client: TestClient, db: Session
) -> None:
    host_id = _register(client)
    _poll(client, host_id)
    db.expire_all()
    started = time.monotonic()
    result = _provisioner(pickup_timeout_seconds=0.15).provision(
        db.get(Host, host_id), lab_tier="standard"
    )
    assert result is not None and result.failed
    assert time.monotonic() - started < 3
    # The withdrawn provision is not delivered later; only the safety teardown is.
    late = _poll(client, host_id)
    assert late.status_code == 200
    assert late.json()["kind"] == "teardown"
    assert _poll(client, host_id).status_code == 204


def test_an_agent_that_takes_a_command_but_never_answers_times_out(
    client: TestClient, db: Session
) -> None:
    host_id = _register(client)
    _poll(client, host_id)
    db.expire_all()
    seen: list = []
    agent = _agent(client, host_id, None, seen)  # takes it, reports nothing
    result = _provisioner(result_timeout_seconds=0.2).provision(
        db.get(Host, host_id), lab_tier="standard"
    )
    agent.join(timeout=5)
    assert seen and result is not None and result.failed


def test_teardown_is_queued_for_a_pull_mode_host(client: TestClient, db: Session) -> None:
    host_id = _register(client)
    _poll(client, host_id)
    db.expire_all()
    _provisioner().teardown(db.get(Host, host_id))
    polled = _poll(client, host_id)
    assert polled.json()["kind"] == "teardown"
    assert polled.json()["body"] == {}


def test_a_host_with_an_agent_url_is_dialled_not_queued(client: TestClient, db: Session) -> None:
    host_id = _register(client, agentUrl="http://127.0.0.1:9")
    _poll(client, host_id)
    db.expire_all()
    result = _provisioner().provision(db.get(Host, host_id), lab_tier="standard")
    assert result is not None and result.failed  # nothing listens on port 9
    assert db.scalars(select(HostCommand)).all() == []


def test_a_host_that_never_polled_has_no_agent_to_ask(client: TestClient, db: Session) -> None:
    host_id = _register(client)
    assert _provisioner().provision(db.get(Host, host_id), lab_tier="standard") is None
    assert db.scalars(select(HostCommand)).all() == []
