from __future__ import annotations

from datetime import datetime, timezone

from fastapi.testclient import TestClient

from matchmaker.config import get_settings
from matchmaker.models import Host
from tests.helpers import (
    FakeHostProvisioner,
    auth_header,
    signup,
)


def test_guest_bootstrap_returns_preauth_and_s3_keys_not_on_session_json(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
) -> None:
    token = signup(client)["accessToken"]
    booked = client.post("/v1/sessions", headers=auth_header(token))
    assert booked.status_code == 201
    body = booked.json()
    assert body["storage"]["savedAsOf"] is None
    assert "accessKeyId" not in body["storage"]
    assert "secretAccessKey" not in body["storage"]
    dumped = str(body)
    assert "test-renterd-secret-key" not in dumped
    assert "test-renterd-access" not in dumped

    assert fake_provisioner.mesh_calls
    hint = fake_provisioner.mesh_calls[-1]
    assert hint is not None
    assert hint.bootstrap_token
    assert hint.bootstrap_origin is None
    assert "preauth" not in (hint.bootstrap_token or "").lower()

    missing = client.post("/v1/guest/bootstrap")
    assert missing.status_code == 401

    wrong = client.post(
        "/v1/guest/bootstrap",
        headers={"Authorization": "Bearer not-a-valid-bootstrap-token-value-32chars"},
    )
    assert wrong.status_code == 404

    redeemed = client.post(
        "/v1/guest/bootstrap",
        headers={"Authorization": f"Bearer {hint.bootstrap_token}"},
    )
    assert redeemed.status_code == 200
    guest = redeemed.json()
    assert guest["preauthKey"] == body["mesh"]["guest"]["preauthKey"]
    assert guest["hostname"] == body["mesh"]["guest"]["hostname"]
    assert "--advertise-exit-node" not in " ".join(guest["tailscaleUp"])
    assert "--advertise-tags" not in " ".join(guest["tailscaleUp"])
    assert guest["storage"]["accessKeyId"] == "test-renterd-access"
    assert guest["storage"]["secretAccessKey"] == "test-renterd-secret-key"
    assert "--s3-env-auth" in guest["storage"]["rcloneArgs"]
    assert hint.bootstrap_token != guest["preauthKey"]


def test_guest_bootstrap_origin_is_on_provision_hint_when_configured(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    monkeypatch,
) -> None:
    monkeypatch.setenv("MATCHMAKER_PUBLIC_ORIGIN", "http://matchmaker.test.invalid")
    get_settings.cache_clear()
    try:
        token = signup(client)["accessToken"]
        booked = client.post("/v1/sessions", headers=auth_header(token))
        assert booked.status_code == 201
        hint = fake_provisioner.mesh_calls[-1]
        assert hint is not None
        assert hint.bootstrap_origin == "http://matchmaker.test.invalid"
        assert hint.bootstrap_token
        assert "preauthKey" not in booked.json()["storage"]
    finally:
        monkeypatch.delenv("MATCHMAKER_PUBLIC_ORIGIN", raising=False)
        get_settings.cache_clear()


def test_saved_as_of_updates_session_storage(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
) -> None:
    token = signup(client)["accessToken"]
    booked = client.post("/v1/sessions", headers=auth_header(token))
    session_id = booked.json()["sessionId"]
    hint = fake_provisioner.mesh_calls[-1]
    assert hint is not None
    stamp = "2026-09-11T21:00:00+00:00"
    reported = client.post(
        "/v1/guest/saved-as-of",
        headers={"Authorization": f"Bearer {hint.bootstrap_token}"},
        json={"savedAsOf": stamp},
    )
    assert reported.status_code == 200
    assert reported.json()["sessionId"] == session_id
    again = client.post("/v1/sessions", headers=auth_header(token))
    assert again.status_code == 200
    saved = again.json()["storage"]["savedAsOf"]
    parsed = datetime.fromisoformat(saved.replace("Z", "+00:00"))
    assert parsed == datetime(2026, 9, 11, 21, 0, tzinfo=timezone.utc)
    assert "secretAccessKey" not in again.json()["storage"]


def test_end_session_tears_down_host_and_expires_bootstrap(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
) -> None:
    token = signup(client)["accessToken"]
    booked = client.post("/v1/sessions", headers=auth_header(token))
    session_id = booked.json()["sessionId"]
    hint = fake_provisioner.mesh_calls[-1]
    assert hint is not None
    ended = client.post(f"/v1/sessions/{session_id}/end", headers=auth_header(token))
    assert ended.status_code == 200
    assert fake_provisioner.teardown_calls == [seeded_host.id]
    expired = client.post(
        "/v1/guest/bootstrap",
        headers={"Authorization": f"Bearer {hint.bootstrap_token}"},
    )
    assert expired.status_code == 404
