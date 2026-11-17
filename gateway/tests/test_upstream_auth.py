"""The gateway logs in to the guest desktop for the renter.

The guest desktop asks for a per-session password that only the guest and
the gateway know, so another machine on the mesh cannot open it. The
browser never sees or sends that password.
"""

from __future__ import annotations

import asyncio
import base64
import http
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from uuid import uuid4

import httpx
import pytest
from fastapi.testclient import TestClient
from starlette.websockets import WebSocketDisconnect

from dcl_gateway.config import get_settings
from dcl_gateway.main import app
from dcl_gateway.proxy import _forward_request_headers, _forward_response_headers
from dcl_gateway.resolve import ResolvedSession, resolve_token

GOOD_TOKEN = "abcdefghijklmnopqrstuvwxyz012345"
PASSWORD = "S3ssionDesktopPassw0rdOnlyGateway"
EXPECTED = "Basic " + base64.b64encode(f"ubuntu:{PASSWORD}".encode()).decode()


def _session(target: str, password: str | None = PASSWORD) -> ResolvedSession:
    return ResolvedSession(
        session_id=uuid4(),
        target_url=target,
        streamer="kasm",
        upstream_user="ubuntu" if password else None,
        upstream_password=password,
    )


def test_authorization_header_is_basic_user_password() -> None:
    assert _session("https://guest.test.example/").upstream_authorization() == EXPECTED
    assert _session("https://guest.test.example/", None).upstream_authorization() is None
    half = ResolvedSession(uuid4(), "https://g.test.example/", "kasm", upstream_user="ubuntu")
    assert half.upstream_authorization() is None


def test_password_is_not_in_the_session_repr() -> None:
    assert PASSWORD not in repr(_session("https://guest.test.example/"))


def test_browser_authorization_is_replaced_not_forwarded() -> None:
    forwarded = _forward_request_headers(
        {"Authorization": "Basic ZXZpbDpldmls", "Accept": "text/html"},
        "https://100.64.0.12:8443/",
        EXPECTED,
    )
    assert forwarded["Authorization"] == EXPECTED
    assert [key for key in forwarded if key.lower() == "authorization"] == ["Authorization"]


def test_without_a_desktop_password_headers_pass_through_unchanged() -> None:
    forwarded = _forward_request_headers(
        {"authorization": "Bearer something"}, "https://100.64.0.12:8443/"
    )
    assert forwarded["authorization"] == "Bearer something"


def test_guest_login_prompt_is_hidden_from_the_browser() -> None:
    headers = {"WWW-Authenticate": 'Basic realm="Websockify"', "Content-Type": "text/plain"}
    hidden = _forward_response_headers(
        headers, "https://g.test.example/", "", hide_auth_challenge=True
    )
    assert "WWW-Authenticate" not in hidden
    assert hidden["Content-Type"] == "text/plain"
    kept = _forward_response_headers(headers, "https://g.test.example/", "")
    assert "WWW-Authenticate" in kept


def test_resolve_reads_the_desktop_login_from_the_matchmaker() -> None:
    session_id = uuid4()

    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            json={
                "sessionId": str(session_id),
                "targetUrl": "https://dcl-guest.test.example:8443/",
                "streamer": "kasm",
                "upstreamUser": "ubuntu",
                "upstreamPassword": PASSWORD,
            },
        )

    resolved = asyncio.run(
        resolve_token(
            matchmaker_url="http://matchmaker.test.invalid",
            resolve_token="test-gateway-resolve-token",
            grant_token=GOOD_TOKEN,
            transport=httpx.MockTransport(handler),
        )
    )
    assert resolved is not None
    assert resolved.upstream_authorization() == EXPECTED


# --- a guest desktop that asks for the password -----------------------------


class _GuestHandler(BaseHTTPRequestHandler):
    seen: list[str | None] = []

    def do_GET(self) -> None:  # noqa: N802
        sent = self.headers.get("Authorization")
        type(self).seen.append(sent)
        if sent != EXPECTED:
            self.send_response(401)
            self.send_header("WWW-Authenticate", 'Basic realm="Websockify"')
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        body = b"desktop-page"
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        return


@pytest.fixture
def guest_http():
    _GuestHandler.seen = []
    server = ThreadingHTTPServer(("127.0.0.1", 0), _GuestHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{server.server_address[1]}/"
    server.shutdown()
    thread.join(timeout=2)


def _client_for(monkeypatch, session: ResolvedSession) -> TestClient:
    async def fake_resolve(*, grant_token: str, **_kwargs):
        return session if grant_token == GOOD_TOKEN else None

    monkeypatch.setattr("dcl_gateway.main.resolve_token", fake_resolve)
    get_settings.cache_clear()
    return TestClient(app)


def test_renter_reaches_a_password_protected_desktop_without_knowing_it(
    monkeypatch, guest_http: str
) -> None:
    with _client_for(monkeypatch, _session(guest_http)) as client:
        page = client.get(f"/s/{GOOD_TOKEN}/index.html")
        assert page.status_code == 200
        assert page.content == b"desktop-page"
        assert PASSWORD not in page.text
        assert all(PASSWORD not in value for value in page.headers.values())
        # A header the browser sends cannot override the gateway's login.
        spoofed = client.get(
            f"/s/{GOOD_TOKEN}/index.html", headers={"Authorization": "Basic ZXZpbDpldmls"}
        )
        assert spoofed.status_code == 200
    assert _GuestHandler.seen == [EXPECTED, EXPECTED]


def test_wrong_desktop_password_is_a_plain_401_with_no_browser_prompt(
    monkeypatch, guest_http: str
) -> None:
    with _client_for(monkeypatch, _session(guest_http, "not-the-guests-password")) as client:
        page = client.get(f"/s/{GOOD_TOKEN}/index.html")
    assert page.status_code == 401
    assert "www-authenticate" not in {key.lower() for key in page.headers}


def test_a_direct_mesh_caller_without_the_password_is_refused(guest_http: str) -> None:
    """What another PC on the mesh gets if it dials the guest itself."""
    direct = httpx.get(guest_http)
    assert direct.status_code == 401


# --- the live desktop stream is a WebSocket ---------------------------------


@pytest.fixture
def guest_ws():
    """A WebSocket guest: needs the password, speaks the `binary` subprotocol."""
    from websockets.asyncio.server import serve

    state: dict = {"port": None, "auth": [], "subprotocols": []}
    ready = threading.Event()
    stop: dict = {}

    def check(connection, request):
        state["auth"].append(request.headers.get("Authorization"))
        if request.headers.get("Authorization") != EXPECTED:
            return connection.respond(http.HTTPStatus.UNAUTHORIZED, "no\n")
        return None

    async def echo(connection) -> None:
        state["subprotocols"].append(connection.subprotocol)
        async for message in connection:
            if isinstance(message, bytes):
                await connection.send(b"echo:" + message)
            else:
                await connection.send("echo:" + message)

    async def main() -> None:
        stop["event"] = asyncio.Event()
        stop["loop"] = asyncio.get_running_loop()
        async with serve(
            echo, "127.0.0.1", 0, subprotocols=["binary"], process_request=check
        ) as server:
            state["port"] = server.sockets[0].getsockname()[1]
            ready.set()
            await stop["event"].wait()

    thread = threading.Thread(target=lambda: asyncio.run(main()), daemon=True)
    thread.start()
    assert ready.wait(5)
    state["url"] = f"http://127.0.0.1:{state['port']}/"
    yield state
    stop["loop"].call_soon_threadsafe(stop["event"].set)
    thread.join(timeout=3)


def test_desktop_stream_passes_with_the_password_and_the_guests_subprotocol(
    monkeypatch, guest_ws: dict
) -> None:
    with _client_for(monkeypatch, _session(guest_ws["url"])) as client:
        with client.websocket_connect(
            f"/s/{GOOD_TOKEN}/websockify", subprotocols=["binary"]
        ) as stream:
            assert stream.accepted_subprotocol == "binary"
            stream.send_bytes(b"\x00\x01frame")
            assert stream.receive_bytes() == b"echo:\x00\x01frame"
            stream.send_text("hello")
            assert stream.receive_text() == "echo:hello"
    assert guest_ws["auth"] == [EXPECTED]
    assert guest_ws["subprotocols"] == ["binary"]


def test_desktop_stream_is_refused_when_the_guest_rejects_the_login(
    monkeypatch, guest_ws: dict
) -> None:
    with _client_for(monkeypatch, _session(guest_ws["url"], "wrong-password-value")) as client:
        with pytest.raises(WebSocketDisconnect):
            with client.websocket_connect(f"/s/{GOOD_TOKEN}/websockify", subprotocols=["binary"]):
                pass
    assert guest_ws["subprotocols"] == []


def test_desktop_stream_unknown_token_is_closed(monkeypatch, guest_ws: dict) -> None:
    with _client_for(monkeypatch, _session(guest_ws["url"])) as client:
        with pytest.raises(WebSocketDisconnect):
            with client.websocket_connect("/s/zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz/websockify"):
                pass
    assert guest_ws["auth"] == []
