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
    mint_desktop_link,
    new_ingress_token,
    redeem_desktop_link,
    public_locator_url,
)
from matchmaker.models import (
    Host,
    LabSession,
    SessionDesktopLink,
    SessionIngressToken,
    SessionStatus,
    User,
)
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
        link = kasm_url[len(f"{GATEWAY_ORIGIN}/s/") :].rstrip("/")
        # The URL holds a single-use link, never the session's own token.
        assert lookup_ingress_token(db, link) is None
        redeemed = redeem_desktop_link(db, link)
        assert redeemed is not None
        assert redeemed.target.target_url == KASM_URL
        assert redeemed.target.session_id == UUID(created.json()["sessionId"])
        assert redeemed.cookie_token != link

        again = client.post("/v1/sessions", headers=auth_header(token))
        assert again.status_code == 200
        # Each answer carries a fresh link to the same session.
        fresh = again.json()["connection"]["kasmUrl"]
        assert fresh.startswith(f"{GATEWAY_ORIGIN}/s/") and fresh != kasm_url
        assert redeem_desktop_link(db, fresh.split("/s/")[1].rstrip("/")).cookie_token == (
            redeemed.cookie_token
        )
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
        link = created.json()["connection"]["kasmUrl"].split("/s/")[1].rstrip("/")
        assert client.post(
            "/v1/gateway/redeem", json={"token": link}
        ).status_code == 401
        redeemed = client.post(
            "/v1/gateway/redeem",
            json={"token": link},
            headers={"Authorization": f"Bearer {GATEWAY_RESOLVE}"},
        )
        assert redeemed.status_code == 200
        assert redeemed.json()["targetUrl"] == KASM_URL
        grant_token = redeemed.json()["cookieToken"]

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
        link = created.json()["connection"]["kasmUrl"].split("/s/")[1].rstrip("/")
        grant_token = redeem_desktop_link(db, link).cookie_token
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


def _active_grant(db: Session, seeded_host: Host) -> SessionIngressToken:
    user = User(email=f"link-{uuid4()}@example.com", password_hash=hash_password("pw-12345678"))
    db.add(user)
    db.commit()
    session = LabSession(
        user_id=user.id,
        host_id=seeded_host.id,
        status=SessionStatus.ACTIVE,
        started_at=datetime.now(timezone.utc),
    )
    db.add(session)
    db.commit()
    return ensure_session_ingress(
        db, session_id=session.id, target_url=LIVE_KASM_URL, streamer="kasm", ttl_seconds=3600
    )


def test_a_desktop_link_works_once_except_for_the_browser_that_used_it(
    db: Session, seeded_host: Host
) -> None:
    """Issue 29: a copied link is no use on another device."""
    grant = _active_grant(db, seeded_host)
    link = mint_desktop_link(db, session_id=grant.session_id)
    first = redeem_desktop_link(db, link)
    assert first is not None and first.cookie_token == grant.token
    # Someone else with the same link (no cookie, or another session's cookie).
    assert redeem_desktop_link(db, link) is None
    assert redeem_desktop_link(db, link, browser_cookie=new_ingress_token()) is None
    # The same browser again (second tab, double click).
    again = redeem_desktop_link(db, link, browser_cookie=first.cookie_token)
    assert again is not None and again.target.session_id == grant.session_id


def test_a_desktop_link_expires_and_dead_links_are_dropped(
    db: Session, seeded_host: Host
) -> None:
    grant = _active_grant(db, seeded_host)
    past = datetime.now(timezone.utc) - timedelta(minutes=10)
    old = mint_desktop_link(db, session_id=grant.session_id, now=past)
    assert redeem_desktop_link(db, old) is None
    mint_desktop_link(db, session_id=grant.session_id)
    remaining = db.scalars(
        select(SessionDesktopLink).where(SessionDesktopLink.session_id == grant.session_id)
    ).all()
    assert len(remaining) == 1
    expire_session_ingress(db, session_id=grant.session_id)
    assert db.scalars(
        select(SessionDesktopLink).where(SessionDesktopLink.session_id == grant.session_id)
    ).all() == []
