"""Asynchronous booking (issue 4): the request returns before the guest boots."""

from __future__ import annotations

from collections.abc import Callable, Generator
from datetime import datetime, timedelta, timezone

import pytest
from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.booking import FAILURE_TIMED_OUT, fail_stuck_provisioning
from matchmaker.deps import get_provision_scheduler
from matchmaker.main import app
from matchmaker.models import Host, LabSession, LedgerEntry, SessionStatus
from tests.helpers import (
    LIVE_KASM_URL,
    PROVISION_URL,
    FakeHeadscale,
    FakeHostProvisioner,
    auth_header,
    signup,
)


class DeferredScheduler:
    """Holds jobs until the test runs them, like a slow host."""

    def __init__(self) -> None:
        self.jobs: list[Callable[[], None]] = []

    def submit(self, job: Callable[[], None]) -> None:
        self.jobs.append(job)

    def run_all(self) -> None:
        jobs, self.jobs = self.jobs, []
        for job in jobs:
            job()


@pytest.fixture
def deferred() -> Generator[DeferredScheduler, None, None]:
    scheduler = DeferredScheduler()
    app.dependency_overrides[get_provision_scheduler] = lambda: scheduler
    yield scheduler
    app.dependency_overrides.pop(get_provision_scheduler, None)


def _book(client: TestClient, email: str = "user@example.com"):
    token = signup(client, email=email)["accessToken"]
    return token, client.post("/v1/sessions", headers=auth_header(token))


def test_booking_returns_202_before_the_agent_is_called(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    deferred: DeferredScheduler,
) -> None:
    token, booked = _book(client)
    assert booked.status_code == 202
    body = booked.json()
    assert body["status"] == "provisioning"
    assert body["connection"] is None
    assert fake_provisioner.calls == []
    assert len(deferred.jobs) == 1

    polled = client.get(f"/v1/sessions/{body['sessionId']}", headers=auth_header(token))
    assert polled.status_code == 200
    assert polled.json()["status"] == "provisioning"

    deferred.run_all()
    ready = client.get(f"/v1/sessions/{body['sessionId']}", headers=auth_header(token))
    assert ready.json()["status"] == "active"
    assert ready.json()["connection"]["kasmUrl"] == LIVE_KASM_URL
    assert fake_provisioner.calls == [(seeded_host.id, "standard")]


def test_a_provisioning_session_occupies_the_host_and_the_user(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    deferred: DeferredScheduler,
) -> None:
    token, booked = _book(client)
    session_id = booked.json()["sessionId"]

    again = client.post("/v1/sessions", headers=auth_header(token))
    assert again.status_code == 202
    assert again.json()["sessionId"] == session_id
    assert len(deferred.jobs) == 1

    _, other = _book(client, email="other@example.com")
    assert other.status_code == 409

    current = client.get("/v1/sessions/current", headers=auth_header(token))
    assert current.status_code == 200
    assert current.json()["sessionId"] == session_id


def test_the_credit_clock_starts_when_the_desktop_is_ready(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    deferred: DeferredScheduler,
    db: Session,
) -> None:
    _, booked = _book(client)
    session = db.get(LabSession, booked.json()["sessionId"])
    session.started_at = datetime.now(timezone.utc) - timedelta(minutes=4)
    db.commit()

    deferred.run_all()
    db.expire_all()
    session = db.get(LabSession, booked.json()["sessionId"])
    assert session.status == SessionStatus.ACTIVE
    assert datetime.now(timezone.utc) - session.started_at < timedelta(seconds=30)


def test_failed_provision_marks_the_session_failed_and_frees_the_host(
    client: TestClient,
    seeded_host: Host,
    deferred: DeferredScheduler,
    fake_headscale: FakeHeadscale,
    db: Session,
) -> None:
    seeded_host.kasm_url = PROVISION_URL
    db.commit()
    token, booked = _book(client)
    session_id = booked.json()["sessionId"]
    deferred.run_all()

    polled = client.get(f"/v1/sessions/{session_id}", headers=auth_header(token))
    body = polled.json()
    assert body["status"] == "failed"
    assert body["failureReason"]
    assert body["connection"] is None
    assert f"dcl-guest-{session_id}" in fake_headscale.expired
    assert db.scalars(select(LedgerEntry)).all() == []
    assert client.get("/v1/sessions/current", headers=auth_header(token)).status_code == 404

    seeded_host.kasm_url = LIVE_KASM_URL
    db.commit()
    retry = client.post("/v1/sessions", headers=auth_header(token))
    assert retry.status_code == 202
    assert retry.json()["sessionId"] != session_id


def test_ending_while_provisioning_is_free_and_stops_the_guest(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    deferred: DeferredScheduler,
    db: Session,
) -> None:
    token, booked = _book(client)
    session_id = booked.json()["sessionId"]
    ended = client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(token))
    assert ended.status_code == 200
    assert ended.json()["creditsDebited"] == 0
    assert db.scalars(select(LedgerEntry)).all() == []

    deferred.run_all()
    db.expire_all()
    assert db.get(LabSession, session_id).status == SessionStatus.ENDED
    # The job found the session already ended, so the host never booted it.
    assert fake_provisioner.calls == []


def test_ending_during_the_boot_tears_the_guest_down_when_it_finishes(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    deferred: DeferredScheduler,
    db: Session,
) -> None:
    token, booked = _book(client)
    session_id = booked.json()["sessionId"]
    real_provision = fake_provisioner.provision

    def provision_while_user_leaves(host, **kwargs):
        ended = client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(token))
        assert ended.status_code == 200
        return real_provision(host, **kwargs)

    fake_provisioner.provision = provision_while_user_leaves  # type: ignore[method-assign]
    deferred.run_all()
    db.expire_all()
    assert db.get(LabSession, session_id).status == SessionStatus.ENDED
    # Once for the end request, once more when the boot finished afterwards.
    assert fake_provisioner.teardown_calls == [seeded_host.id, seeded_host.id]
    assert db.scalars(select(LedgerEntry)).all() == []


def test_a_crashing_agent_call_fails_the_session_instead_of_hanging(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    deferred: DeferredScheduler,
    db: Session,
) -> None:
    def boom(*_args, **_kwargs):
        raise RuntimeError("agent exploded")

    fake_provisioner.provision = boom  # type: ignore[method-assign]
    _, booked = _book(client)
    deferred.run_all()
    db.expire_all()
    assert db.get(LabSession, booked.json()["sessionId"]).status == SessionStatus.FAILED


def test_stuck_provisioning_sessions_time_out(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    deferred: DeferredScheduler,
    fake_headscale: FakeHeadscale,
    db: Session,
) -> None:
    _, booked = _book(client)
    session_id = booked.json()["sessionId"]
    later = datetime.now(timezone.utc) + timedelta(seconds=500)

    assert fail_stuck_provisioning(db, timeout_seconds=600, headscale=fake_headscale, now=later) == []
    failed = fail_stuck_provisioning(
        db,
        timeout_seconds=420,
        headscale=fake_headscale,
        provisioner=fake_provisioner,
        now=later,
    )
    assert [str(item) for item in failed] == [session_id]
    db.expire_all()
    row = db.get(LabSession, session_id)
    assert row.status == SessionStatus.FAILED
    assert row.failure_reason == FAILURE_TIMED_OUT
    assert fake_provisioner.teardown_calls == [seeded_host.id]

    # The late job must not resurrect it.
    deferred.run_all()
    db.expire_all()
    assert db.get(LabSession, session_id).status == SessionStatus.FAILED


def test_another_users_session_is_not_readable(
    client: TestClient, seeded_host: Host, fake_provisioner: FakeHostProvisioner
) -> None:
    _, booked = _book(client)
    stranger = signup(client, email="stranger@example.com")["accessToken"]
    response = client.get(
        f"/v1/sessions/{booked.json()['sessionId']}", headers=auth_header(stranger)
    )
    assert response.status_code == 404


def test_a_failed_boot_is_not_hidden_by_the_previous_sessions_link(
    client: TestClient, seeded_host: Host, db: Session
) -> None:
    """Found by the integration test: the host row keeps the last desktop URL.

    When the agent was asked and failed, that old URL must not make the new
    booking look ready.
    """
    from matchmaker.deps import get_host_provisioner
    from matchmaker.provision import ProvisionResult

    assert seeded_host.kasm_url.startswith("https://")  # a live-looking URL

    class FailingAgent:
        def provision(self, host, **_kwargs):
            return ProvisionResult(connection_url="", failed=True)

        def teardown(self, host) -> None:
            pass

    app.dependency_overrides[get_host_provisioner] = lambda: FailingAgent()
    try:
        token = signup(client)["accessToken"]
        booked = client.post("/v1/sessions", headers=auth_header(token))
    finally:
        app.dependency_overrides.pop(get_host_provisioner, None)
    assert booked.status_code == 502
    db.expire_all()
    assert [row.status for row in db.scalars(select(LabSession)).all()] == ["failed"]


def test_a_host_without_an_agent_keeps_its_fixed_desktop_url(
    client: TestClient, seeded_host: Host
) -> None:
    """`None` means there was no agent to ask; the fixed URL on the row is used."""
    from matchmaker.deps import get_host_provisioner

    class NoAgent:
        def provision(self, host, **_kwargs):
            return None

        def teardown(self, host) -> None:
            pass

    app.dependency_overrides[get_host_provisioner] = lambda: NoAgent()
    try:
        token = signup(client)["accessToken"]
        booked = client.post("/v1/sessions", headers=auth_header(token))
    finally:
        app.dependency_overrides.pop(get_host_provisioner, None)
    assert booked.status_code == 201
    assert booked.json()["connection"]["kasmUrl"] == seeded_host.kasm_url
