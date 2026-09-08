from __future__ import annotations

from datetime import datetime, timedelta, timezone
from uuid import UUID, uuid4

from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.auth import hash_password
from matchmaker.config import get_settings
from matchmaker.ingress import (
    TOKEN_RE,
    ensure_session_ingress,
    expire_session_ingress,
    hash_ingress_token,
    lookup_ingress_token,
    new_ingress_token,
    public_locator_url,
)
from matchmaker.models import Host, LabSession, SessionIngressToken, SessionStatus, User
from tests.helpers import (
    KASM_URL,
    LIVE_KASM_URL,
    LIVE_SUNSHINE_URL,
    auth_header,
    signup,
)


GATEWAY_ORIGIN = "https://gateway.test.example"
GATEWAY_RESOLVE = "test-gateway-resolve-token"


def _enable_gateway(monkeypatch) -> None:
    monkeypatch.setenv("GATEWAY_PUBLIC_ORIGIN", GATEWAY_ORIGIN)
    monkeypatch.setenv("GATEWAY_RESOLVE_TOKEN", GATEWAY_RESOLVE)
    get_settings.cache_clear()


def _disable_gateway(monkeypatch) -> None:
    monkeypatch.delenv("GATEWAY_PUBLIC_ORIGIN", raising=False)
    monkeypatch.delenv("GATEWAY_RESOLVE_TOKEN", raising=False)
    get_settings.cache_clear()


def test_new_ingress_token_is_ungessable_and_hashed() -> None:
    token = new_ingress_token()
    assert TOKEN_RE.fullmatch(token)
    digest = hash_ingress_token(token)
    assert digest != token
    assert len(digest) == 64
    assert hash_ingress_token(token) == digest
    assert hash_ingress_token(token + "x") != digest
    other = new_ingress_token()
    assert other != token


def test_lookup_wrong_expired_and_other_session_fail_closed(db: Session, seeded_host: Host) -> None:
    user_id = uuid4()
    user = User(id=user_id, email="ingress@example.com", password_hash=hash_password("password123"))
    db.add(user)
    session = LabSession(
        user_id=user_id,
        host_id=seeded_host.id,
        status=SessionStatus.ACTIVE,
        started_at=datetime.now(timezone.utc),
    )
    db.add(session)
    db.commit()
    db.refresh(session)

    grant = ensure_session_ingress(
        db,
        session_id=session.id,
        target_url=LIVE_KASM_URL,
        streamer="kasm",
        ttl_seconds=3600,
    )
    found = lookup_ingress_token(db, grant.token)
    assert found is not None
    assert found.session_id == session.id
    assert found.target_url == LIVE_KASM_URL
    assert found.streamer == "kasm"

    assert lookup_ingress_token(db, "not-a-valid-token-value-at-all-nope") is None
    assert lookup_ingress_token(db, new_ingress_token()) is None

    other = LabSession(
        user_id=user_id,
        host_id=seeded_host.id,
        status=SessionStatus.ENDED,
        started_at=datetime.now(timezone.utc),
        ended_at=datetime.now(timezone.utc),
    )
    db.add(other)
    db.commit()
    db.refresh(other)
    other_grant = ensure_session_ingress(
        db,
        session_id=other.id,
        target_url="https://other-guest.test.example/",
        streamer="kasm",
        ttl_seconds=3600,
    )
    # Ended session token must not resolve, and must not return the active session.
    missed = lookup_ingress_token(db, other_grant.token)
    assert missed is None
    still = lookup_ingress_token(db, grant.token)
    assert still is not None
    assert still.session_id == session.id
    assert still.target_url == LIVE_KASM_URL

    expired = db.scalars(
        select(SessionIngressToken).where(SessionIngressToken.session_id == session.id)
    ).first()
    assert expired is not None
    expired.expires_at = datetime.now(timezone.utc) - timedelta(seconds=1)
    db.commit()
    assert lookup_ingress_token(db, grant.token) is None


def test_session_json_keeps_mesh_locator_when_gateway_unset(
    client: TestClient, seeded_host: Host
) -> None:
    token = signup(client)["accessToken"]
    response = client.post("/v1/sessions", headers=auth_header(token))
    assert response.status_code == 201
    assert response.json()["connection"]["kasmUrl"] == KASM_URL
    assert GATEWAY_ORIGIN not in response.json()["connection"]["kasmUrl"]
    assert seeded_host.id


def test_session_json_returns_public_gateway_url_when_configured(
    client: TestClient, seeded_host: Host, monkeypatch, db: Session
) -> None:
    _enable_gateway(monkeypatch)
    try:
        token = signup(client)["accessToken"]
        created = client.post("/v1/sessions", headers=auth_header(token))
        assert created.status_code == 201
        kasm_url = created.json()["connection"]["kasmUrl"]
        assert kasm_url.startswith(f"{GATEWAY_ORIGIN}/s/")
        assert kasm_url.endswith("/")
        assert LIVE_KASM_URL not in kasm_url
        assert KASM_URL not in kasm_url
        grant_token = kasm_url[len(f"{GATEWAY_ORIGIN}/s/") :].rstrip("/")
        target = lookup_ingress_token(db, grant_token)
        assert target is not None
        assert target.target_url == KASM_URL
        assert target.session_id == UUID(created.json()["sessionId"])

        again = client.post("/v1/sessions", headers=auth_header(token))
        assert again.status_code == 200
        assert again.json()["connection"]["kasmUrl"] == kasm_url
    finally:
        _disable_gateway(monkeypatch)


def test_pro_session_wraps_sunshine_url_not_moonlight_host(
    client: TestClient, make_approved_host, fake_provisioner, monkeypatch
) -> None:
    _enable_gateway(monkeypatch)
    try:
        fake_provisioner.connection_url = LIVE_SUNSHINE_URL
        make_approved_host(
            display_name="pro-host",
            lab_tier="pro",
            share_gpu=True,
            shared_gpu_id="gpu-0",
            kasm_url=LIVE_SUNSHINE_URL,
        )
        token = signup(client)["accessToken"]
        created = client.post(
            "/v1/sessions",
            headers=auth_header(token),
            json={"labTier": "pro"},
        )
        assert created.status_code == 201
        connection = created.json()["connection"]
        assert connection["streamer"] == "sunshine"
        assert connection["kasmUrl"] is None
        assert connection["sunshineUrl"].startswith(f"{GATEWAY_ORIGIN}/s/")
        assert connection["moonlight"]["host"] == "dcl-guest.test.example"
    finally:
        _disable_gateway(monkeypatch)


def test_resolve_endpoint_is_scoped_and_fail_closed(
    client: TestClient, seeded_host: Host, monkeypatch
) -> None:
    _enable_gateway(monkeypatch)
    try:
        user_token = signup(client)["accessToken"]
        created = client.post("/v1/sessions", headers=auth_header(user_token))
        grant_token = created.json()["connection"]["kasmUrl"].split("/s/")[1].rstrip("/")

        denied = client.post("/v1/gateway/resolve", json={"token": grant_token})
        assert denied.status_code == 401

        wrong_bearer = client.post(
            "/v1/gateway/resolve",
            json={"token": grant_token},
            headers={"Authorization": "Bearer definitely-not-the-resolve-token"},
        )
        assert wrong_bearer.status_code == 401
        assert grant_token not in wrong_bearer.text
        assert KASM_URL not in wrong_bearer.text

        ok = client.post(
            "/v1/gateway/resolve",
            json={"token": grant_token},
            headers={"Authorization": f"Bearer {GATEWAY_RESOLVE}"},
        )
        assert ok.status_code == 200
        body = ok.json()
        assert body["sessionId"] == created.json()["sessionId"]
        assert body["targetUrl"] == KASM_URL
        assert body["streamer"] == "kasm"

        missing = client.post(
            "/v1/gateway/resolve",
            json={"token": new_ingress_token()},
            headers={"Authorization": f"Bearer {GATEWAY_RESOLVE}"},
        )
        assert missing.status_code == 404
        assert missing.json()["detail"] == "Session not found"
        assert "kasmUrl" not in missing.text
        assert KASM_URL not in missing.text
    finally:
        _disable_gateway(monkeypatch)


def test_end_session_invalidates_gateway_token(
    client: TestClient, seeded_host: Host, monkeypatch, db: Session
) -> None:
    _enable_gateway(monkeypatch)
    try:
        user_token = signup(client)["accessToken"]
        created = client.post("/v1/sessions", headers=auth_header(user_token))
        session_id = created.json()["sessionId"]
        grant_token = created.json()["connection"]["kasmUrl"].split("/s/")[1].rstrip("/")
        ended = client.post(
            f"/v1/sessions/{session_id}/end", headers=auth_header(user_token)
        )
        assert ended.status_code == 200
        assert lookup_ingress_token(db, grant_token) is None
        resolve = client.post(
            "/v1/gateway/resolve",
            json={"token": grant_token},
            headers={"Authorization": f"Bearer {GATEWAY_RESOLVE}"},
        )
        assert resolve.status_code == 404
        assert seeded_host.id
    finally:
        _disable_gateway(monkeypatch)


def test_public_locator_url_shape() -> None:
    token = "abcdefghijklmnopqrstuvwxyz0123456789ABCD"
    assert public_locator_url("https://gateway.test.example/", token) == (
        f"https://gateway.test.example/s/{token}/"
    )


def test_expire_session_ingress_is_idempotent(db: Session, seeded_host: Host) -> None:
    session_id = uuid4()
    assert expire_session_ingress(db, session_id=session_id) == 0
    assert seeded_host.id
