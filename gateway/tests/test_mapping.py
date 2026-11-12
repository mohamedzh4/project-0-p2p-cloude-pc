from __future__ import annotations

from uuid import uuid4

import httpx
import pytest
from fastapi.testclient import TestClient

from dcl_gateway.config import get_settings
from dcl_gateway.main import app
from dcl_gateway.resolve import ResolvedSession
from dcl_gateway.ssrf import origin_is_allowed, target_is_allowed


GOOD_TOKEN = "abcdefghijklmnopqrstuvwxyz012345"
OTHER_TOKEN = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"


def test_target_allows_https_mesh_and_rejects_placeholders() -> None:
    assert target_is_allowed(
        "https://dcl-guest-aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa/",
        allow_loopback=False,
    )
    assert not target_is_allowed("https://example.invalid/kasm", allow_loopback=False)
    assert not target_is_allowed("http://dcl-guest.example/", allow_loopback=False)
    assert not target_is_allowed("https://127.0.0.1/", allow_loopback=False)
    assert target_is_allowed("http://127.0.0.1:47990/", allow_loopback=True)
    assert not target_is_allowed(
        "https://169.254.169.254/", allow_loopback=False
    )
    assert not target_is_allowed(
        "https://user:pass@dcl-guest.example/", allow_loopback=False
    )
    assert target_is_allowed("https://100.64.1.20:8443/", allow_loopback=False)
    assert not target_is_allowed("https://8.8.8.8/", allow_loopback=False)
    assert not target_is_allowed("https://10.0.0.5/", allow_loopback=False)


def test_websocket_origin_must_match_desktop_origin() -> None:
    assert origin_is_allowed(None, None) is True
    assert origin_is_allowed("https://evil.example", None) is True
    assert (
        origin_is_allowed(
            "https://desktop.lab.example.com",
            "https://desktop.lab.example.com",
        )
        is True
    )
    assert (
        origin_is_allowed(
            "https://storage.lab.example.com",
            "https://desktop.lab.example.com",
        )
        is False
    )
    assert origin_is_allowed(None, "https://desktop.lab.example.com") is False


def test_default_target_env_does_not_make_a_backend(monkeypatch) -> None:
    monkeypatch.setenv("GATEWAY_DEFAULT_TARGET", "https://evil.example/")
    assert not target_is_allowed("https://example.invalid/", allow_loopback=False)
    assert target_is_allowed("https://dcl-guest.test.example/", allow_loopback=False)


@pytest.fixture
def gateway_client(monkeypatch) -> TestClient:
    session_id = uuid4()
    target = {"url": "https://dcl-guest.test.example/"}

    async def fake_resolve(*, grant_token: str, **_kwargs):
        if grant_token == GOOD_TOKEN:
            return ResolvedSession(
                session_id=session_id,
                target_url=target["url"],
                streamer="kasm",
            )
        return None

    monkeypatch.setattr("dcl_gateway.main.resolve_token", fake_resolve)
    get_settings.cache_clear()
    with TestClient(app) as client:
        yield client


def test_unknown_token_is_404_not_a_default_session(gateway_client: TestClient) -> None:
    response = gateway_client.get(f"/s/{OTHER_TOKEN}/")
    assert response.status_code == 404
    assert response.json()["detail"] == "Session not found"
    assert "dcl-guest" not in response.text


def test_valid_token_bootstraps_secure_desktop_cookie(gateway_client: TestClient) -> None:
    response = gateway_client.get(f"/s/{GOOD_TOKEN}/", follow_redirects=False)
    assert response.status_code == 303
    assert response.headers["location"] == "/"
    cookie = response.headers["set-cookie"]
    assert f"dcl_session={GOOD_TOKEN}" in cookie
    assert "HttpOnly" in cookie
    assert "Secure" in cookie
    assert "SameSite=lax" in cookie

    # The fake mesh backend is intentionally unreachable, but the cookie was
    # accepted and resolved; an unknown/missing token would have returned 404.
    proxied = gateway_client.get(
        "/api/__healthcheck",
        headers={"Cookie": f"dcl_session={GOOD_TOKEN}"},
    )
    assert proxied.status_code == 502


def test_rate_limit_failed_lookups_still_fail_closed(gateway_client: TestClient) -> None:
    for _ in range(5):
        denied = gateway_client.get(f"/s/{OTHER_TOKEN}/")
        assert denied.status_code == 404
    limited = gateway_client.get(f"/s/{OTHER_TOKEN}/")
    assert limited.status_code == 404
    assert limited.json()["detail"] == "Session not found"
    # A valid token after the IP is limited also fails closed (no fallthrough).
    blocked_valid = gateway_client.get(f"/s/{GOOD_TOKEN}/")
    assert blocked_valid.status_code == 404


def test_resolve_http_client_maps_200_and_404() -> None:
    from dcl_gateway.resolve import resolve_token

    session_id = uuid4()

    def handler(request: httpx.Request) -> httpx.Response:
        assert request.url.path == "/v1/gateway/resolve"
        assert request.headers["Authorization"] == "Bearer test-gateway-resolve-token"
        import json

        body = json.loads(request.content)
        if body["token"] == GOOD_TOKEN:
            return httpx.Response(
                200,
                json={
                    "sessionId": str(session_id),
                    "targetUrl": "https://dcl-guest.test.example/",
                    "streamer": "kasm",
                },
            )
        return httpx.Response(404, json={"detail": "Session not found"})

    transport = httpx.MockTransport(handler)

    async def _run():
        ok = await resolve_token(
            matchmaker_url="http://matchmaker.test.invalid",
            resolve_token="test-gateway-resolve-token",
            grant_token=GOOD_TOKEN,
            transport=transport,
        )
        assert ok is not None
        assert ok.session_id == session_id
        assert ok.target_url == "https://dcl-guest.test.example/"
        missed = await resolve_token(
            matchmaker_url="http://matchmaker.test.invalid",
            resolve_token="test-gateway-resolve-token",
            grant_token=OTHER_TOKEN,
            transport=transport,
        )
        assert missed is None

    import asyncio

    asyncio.run(_run())
