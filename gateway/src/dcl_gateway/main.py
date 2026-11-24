"""Public HTTPS reverse proxy. Token in the path; no default backend."""

from __future__ import annotations

from contextlib import asynccontextmanager
import hashlib
import logging
from typing import AsyncIterator

import httpx
from fastapi import FastAPI, Request, WebSocket
from fastapi.responses import JSONResponse, RedirectResponse, Response

from dcl_gateway.config import get_settings
from dcl_gateway.proxy import proxy_http, proxy_websocket
from dcl_gateway.rate_limit import FailClosedLimiter
from dcl_gateway.resolve import redeem_link, resolve_token
from dcl_gateway.ssrf import origin_is_allowed, target_is_allowed

logger = logging.getLogger("dcl_gateway")

SESSION_COOKIE = "dcl_session"
_limiter: FailClosedLimiter | None = None
_http_client: httpx.AsyncClient | None = None


def _token_log_id(token: str) -> str:
    return hashlib.sha256(token.encode("utf-8")).hexdigest()[:12]


def _client_ip(request: Request) -> str:
    if request.client is None:
        return "unknown"
    return request.client.host


@asynccontextmanager
async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
    global _limiter, _http_client
    settings = get_settings()
    _limiter = FailClosedLimiter(
        limit=settings.fail_limit,
        window_seconds=float(settings.fail_window_seconds),
    )
    # Guest Kasm/Sunshine certs are mesh-internal (often self-signed). The
    # public internet leg is TLS at Caddy, not this hop.
    _http_client = httpx.AsyncClient(
        timeout=60.0,
        follow_redirects=False,
        verify=False,
    )
    yield
    if _http_client is not None:
        await _http_client.aclose()
        _http_client = None


def create_app() -> FastAPI:
    get_settings()
    app = FastAPI(title="DCL Gateway", version="0.1.0", lifespan=lifespan)

    @app.api_route("/s/{token}", methods=["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"])
    @app.api_route(
        "/s/{token}/{path:path}",
        methods=["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"],
    )
    async def http_ingress(
        request: Request, token: str, path: str = ""
    ) -> Response:
        if request.method in {"GET", "HEAD"} and not path:
            return await _bootstrap_http(request, token)
        return await _handle_http(request, token, path)

    @app.websocket("/s/{token}")
    @app.websocket("/s/{token}/{path:path}")
    async def ws_ingress(websocket: WebSocket, token: str, path: str = "") -> None:
        await _handle_ws(websocket, token, path)

    @app.api_route("/", methods=["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"])
    @app.api_route(
        "/{path:path}",
        methods=["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"],
    )
    async def cookie_http_ingress(request: Request, path: str = "") -> Response:
        return await _handle_http(request, request.cookies.get(SESSION_COOKIE, ""), path)

    @app.websocket("/")
    @app.websocket("/{path:path}")
    async def cookie_ws_ingress(websocket: WebSocket, path: str = "") -> None:
        await _handle_ws(websocket, websocket.cookies.get(SESSION_COOKIE, ""), path)

    return app


async def _bootstrap_http(request: Request, token: str) -> Response:
    settings = get_settings()
    limiter = _limiter
    if limiter is None:
        return JSONResponse({"detail": "unavailable"}, status_code=503)
    ip = _client_ip(request)
    if limiter.blocked(ip):
        return JSONResponse({"detail": "Session not found"}, status_code=404)
    # The URL holds a single-use desktop link (issue 29). The session's own
    # token only ever travels in this HttpOnly cookie.
    redeemed = await redeem_link(
        matchmaker_url=settings.matchmaker_url,
        resolve_token=settings.resolve_token,
        link_token=token,
        cookie_token=request.cookies.get(SESSION_COOKIE) or None,
    )
    if redeemed is None or not target_is_allowed(
        redeemed[0].target_url, allow_loopback=settings.allow_loopback_target
    ):
        limiter.record_failure(ip)
        return JSONResponse({"detail": "Session not found"}, status_code=404)

    response = RedirectResponse(url="/", status_code=303)
    response.set_cookie(
        SESSION_COOKIE,
        redeemed[1],
        secure=True,
        httponly=True,
        samesite="lax",
        path="/",
    )
    return response


async def _handle_http(request: Request, token: str, path: str) -> Response:
    settings = get_settings()
    limiter = _limiter
    client = _http_client
    if limiter is None or client is None:
        return JSONResponse({"detail": "unavailable"}, status_code=503)

    ip = _client_ip(request)
    if limiter.blocked(ip):
        logger.warning("gateway deny rate_limited ip=%s", ip)
        return JSONResponse({"detail": "Session not found"}, status_code=404)

    resolved = await resolve_token(
        matchmaker_url=settings.matchmaker_url,
        resolve_token=settings.resolve_token,
        grant_token=token,
    )
    if resolved is None:
        limiter.record_failure(ip)
        logger.warning(
            "gateway deny unknown_or_expired token=%s ip=%s",
            _token_log_id(token),
            ip,
        )
        return JSONResponse({"detail": "Session not found"}, status_code=404)

    if not target_is_allowed(
        resolved.target_url, allow_loopback=settings.allow_loopback_target
    ):
        limiter.record_failure(ip)
        logger.warning(
            "gateway deny disallowed_target token=%s ip=%s",
            _token_log_id(token),
            ip,
        )
        return JSONResponse({"detail": "Session not found"}, status_code=404)

    public_prefix = settings.public_origin or ""
    logger.info(
        "gateway allow token=%s session=%s ip=%s",
        _token_log_id(token),
        resolved.session_id,
        ip,
    )
    return await proxy_http(
        request,
        target_url=resolved.target_url,
        suffix=path,
        public_prefix=public_prefix,
        client=client,
        allow_loopback=settings.allow_loopback_target,
        upstream_authorization=resolved.upstream_authorization(),
    )


async def _handle_ws(websocket: WebSocket, token: str, path: str) -> None:
    settings = get_settings()
    limiter = _limiter
    ip = websocket.client.host if websocket.client is not None else "unknown"
    if not origin_is_allowed(
        websocket.headers.get("origin"), settings.public_origin
    ):
        await websocket.close(code=4403)
        return
    if limiter is not None and limiter.blocked(ip):
        await websocket.close(code=4404)
        return
    resolved = await resolve_token(
        matchmaker_url=settings.matchmaker_url,
        resolve_token=settings.resolve_token,
        grant_token=token,
    )
    if resolved is None:
        if limiter is not None:
            limiter.record_failure(ip)
        await websocket.close(code=4404)
        return
    if not target_is_allowed(
        resolved.target_url, allow_loopback=settings.allow_loopback_target
    ):
        if limiter is not None:
            limiter.record_failure(ip)
        await websocket.close(code=4404)
        return
    await proxy_websocket(
        websocket,
        target_url=resolved.target_url,
        suffix=path,
        query=websocket.url.query,
        upstream_authorization=resolved.upstream_authorization(),
    )


app = create_app()
