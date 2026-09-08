from __future__ import annotations

from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import Thread
from uuid import uuid4

from fastapi.testclient import TestClient

from dcl_gateway.config import get_settings
from dcl_gateway.main import app
from dcl_gateway.proxy import join_backend, rewrite_location
from dcl_gateway.rate_limit import FailClosedLimiter
from dcl_gateway.resolve import ResolvedSession

GOOD_TOKEN = "abcdefghijklmnopqrstuvwxyz012345"


def test_join_backend_strips_gateway_prefix() -> None:
    assert (
        join_backend("https://dcl-guest.test.example/", "desktop", "")
        == "https://dcl-guest.test.example/desktop"
    )
    assert (
        join_backend("https://dcl-guest.test.example/", "", "x=1")
        == "https://dcl-guest.test.example/?x=1"
    )


def test_rewrite_location_stays_on_public_prefix() -> None:
    rewritten = rewrite_location(
        "https://dcl-guest.test.example/login",
        "https://dcl-guest.test.example/",
        "https://gateway.test.example/s/tok",
    )
    assert rewritten == "https://gateway.test.example/s/tok/login"


def test_rate_limiter_blocks_after_limit() -> None:
    limiter = FailClosedLimiter(limit=3, window_seconds=60)
    assert limiter.blocked("1.1.1.1", now=10.0) is False
    limiter.record_failure("1.1.1.1", now=10.0)
    limiter.record_failure("1.1.1.1", now=11.0)
    limiter.record_failure("1.1.1.1", now=12.0)
    assert limiter.blocked("1.1.1.1", now=12.5) is True
    assert limiter.blocked("2.2.2.2", now=12.5) is False


def test_http_proxy_uses_resolved_target_only(monkeypatch) -> None:
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802
            body = b"kasm-ok" + self.path.encode("ascii")
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *args: object) -> None:
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    port = server.server_address[1]
    backend = f"http://127.0.0.1:{port}/"
    session_id = uuid4()

    async def fake_resolve(*, grant_token: str, **_kwargs):
        if grant_token == GOOD_TOKEN:
            return ResolvedSession(
                session_id=session_id,
                target_url=backend,
                streamer="kasm",
            )
        return None

    monkeypatch.setattr("dcl_gateway.main.resolve_token", fake_resolve)
    get_settings.cache_clear()
    try:
        with TestClient(app) as client:
            ok = client.get(f"/s/{GOOD_TOKEN}/hello")
            assert ok.status_code == 200
            assert ok.content == b"kasm-ok/hello"
            missed = client.get("/s/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/hello")
            assert missed.status_code == 404
            assert b"kasm-ok" not in missed.content
    finally:
        server.shutdown()
        thread.join(timeout=2)
