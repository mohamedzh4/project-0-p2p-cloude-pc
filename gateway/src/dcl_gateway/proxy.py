"""HTTP/WebSocket reverse proxy to a resolved session locator."""

from __future__ import annotations

from collections.abc import Mapping
from urllib.parse import urljoin, urlparse
import logging
import ssl

import httpx
from fastapi import Request, WebSocket, WebSocketDisconnect
from fastapi.responses import Response

from dcl_gateway.ssrf import target_is_allowed

logger = logging.getLogger(__name__)

_HOP_BY_HOP = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "host",
}


def join_backend(target_url: str, suffix: str, query: str) -> str | None:
    parsed_target = urlparse(target_url)
    if not parsed_target.scheme or not parsed_target.netloc:
        return None
    candidate = suffix.strip()
    if candidate.startswith("//") or urlparse(candidate).scheme:
        return None
    path = candidate.lstrip("/")
    if path.startswith("//") or urlparse(path).scheme:
        return None
    base = target_url if target_url.endswith("/") else f"{target_url}/"
    joined = urljoin(base, path)
    if query:
        joiner = "&" if urlparse(joined).query else "?"
        joined = f"{joined}{joiner}{query}"
    parsed_joined = urlparse(joined)
    if (parsed_joined.scheme, parsed_joined.netloc) != (
        parsed_target.scheme,
        parsed_target.netloc,
    ):
        return None
    return joined


def rewrite_location(location: str, target_url: str, public_prefix: str) -> str:
    target_origin = _origin(target_url)
    if not target_origin:
        return location
    if location.startswith(target_origin):
        rest = location[len(target_origin) :]
        return f"{public_prefix.rstrip('/')}{rest}"
    return location


def _origin(url: str) -> str:
    parsed = urlparse(url)
    if not parsed.scheme or not parsed.netloc:
        return ""
    return f"{parsed.scheme}://{parsed.netloc}"


_SESSION_COOKIE = "dcl_session"


def _strip_named_cookie(header: str, name: str) -> str | None:
    kept: list[str] = []
    prefix = f"{name.lower()}="
    for item in header.split(";"):
        piece = item.strip()
        if not piece:
            continue
        if piece.lower().startswith(prefix):
            continue
        kept.append(piece)
    if not kept:
        return None
    return "; ".join(kept)


def _forward_request_headers(
    headers: Mapping[str, str],
    target_url: str,
    upstream_authorization: str | None = None,
) -> dict[str, str]:
    """Headers for the guest. With `upstream_authorization` the gateway logs
    in to the guest desktop itself and whatever the browser sent is dropped."""
    parsed = urlparse(target_url)
    forwarded: dict[str, str] = {}
    for key, value in headers.items():
        lowered = key.lower()
        if lowered in _HOP_BY_HOP:
            continue
        if lowered == "authorization" and upstream_authorization is not None:
            continue
        if lowered == "cookie":
            stripped = _strip_named_cookie(value, _SESSION_COOKIE)
            if stripped:
                forwarded[key] = stripped
            continue
        forwarded[key] = value
    if parsed.netloc:
        forwarded["Host"] = parsed.netloc
    forwarded["X-Forwarded-Proto"] = "https"
    if upstream_authorization is not None:
        forwarded["Authorization"] = upstream_authorization
    return forwarded


def _forward_response_headers(
    headers: Mapping[str, str],
    target_url: str,
    public_prefix: str,
    hide_auth_challenge: bool = False,
) -> dict[str, str]:
    out: dict[str, str] = {}
    for key, value in headers.items():
        lowered = key.lower()
        if lowered in _HOP_BY_HOP:
            continue
        if lowered == "www-authenticate" and hide_auth_challenge:
            # The renter has no desktop password to type; do not prompt.
            continue
        if lowered == "location":
            out[key] = rewrite_location(value, target_url, public_prefix)
            continue
        if lowered == "set-cookie":
            out[key] = _strip_cookie_domain(value)
            continue
        out[key] = value
    return out


def _strip_cookie_domain(value: str) -> str:
    parts = []
    for item in value.split(";"):
        stripped = item.strip()
        if stripped.lower().startswith("domain="):
            continue
        parts.append(stripped)
    return "; ".join(parts)


async def proxy_http(
    request: Request,
    *,
    target_url: str,
    suffix: str,
    public_prefix: str,
    client: httpx.AsyncClient,
    allow_loopback: bool,
    upstream_authorization: str | None = None,
) -> Response:
    backend = join_backend(target_url, suffix, request.url.query)
    if backend is None or not target_is_allowed(
        backend, allow_loopback=allow_loopback
    ):
        logger.warning("gateway proxy: refused backend join")
        return Response(status_code=400, content=b"")
    headers = _forward_request_headers(request.headers, target_url, upstream_authorization)
    body = await request.body()
    try:
        upstream = await client.request(
            request.method,
            backend,
            headers=headers,
            content=body if body else None,
        )
    except httpx.HTTPError:
        logger.warning("gateway proxy: backend unreachable")
        return Response(status_code=502, content=b"")

    response_headers = _forward_response_headers(
        upstream.headers,
        target_url,
        public_prefix,
        hide_auth_challenge=upstream_authorization is not None,
    )
    return Response(
        content=upstream.content,
        status_code=upstream.status_code,
        headers=response_headers,
    )


async def proxy_websocket(
    websocket: WebSocket,
    *,
    target_url: str,
    suffix: str,
    query: str,
    upstream_authorization: str | None = None,
) -> None:
    """Relay a browser WebSocket to the guest.

    The guest is dialled first, so the browser is offered exactly the
    subprotocol the guest chose (the desktop client asks for `binary`), and a
    guest that refuses the connection is reported before the browser's
    handshake completes.
    """
    import asyncio

    import websockets

    backend = join_backend(target_url, suffix, query)
    if backend is None:
        await websocket.close(code=4400)
        return
    parsed = urlparse(backend)
    if parsed.scheme == "https":
        ws_url = backend.replace("https://", "wss://", 1)
    elif parsed.scheme == "http":
        ws_url = backend.replace("http://", "ws://", 1)
    else:
        await websocket.close(code=4400)
        return

    tls: ssl.SSLContext | None = None
    if parsed.scheme == "https":
        # Kasm/Sunshine use a mesh-internal self-signed certificate. The
        # browser-facing connection is independently protected by Caddy TLS.
        tls = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        tls.check_hostname = False
        tls.verify_mode = ssl.CERT_NONE

    requested = [str(item) for item in websocket.scope.get("subprotocols") or []]
    extra_headers: dict[str, str] = {}
    if upstream_authorization is not None:
        extra_headers["Authorization"] = upstream_authorization

    try:
        backend_ws = await websockets.connect(
            ws_url,
            ssl=tls,
            open_timeout=10,
            subprotocols=requested or None,
            additional_headers=extra_headers or None,
            max_size=None,
        )
    except Exception:
        logger.warning("gateway proxy: websocket backend unreachable")
        try:
            await websocket.close(code=1014)
        except Exception:
            pass
        return

    try:
        await websocket.accept(subprotocol=backend_ws.subprotocol)

        async def client_to_backend() -> None:
            try:
                while True:
                    message = await websocket.receive()
                    if message["type"] == "websocket.disconnect":
                        return
                    if "text" in message and message["text"] is not None:
                        await backend_ws.send(message["text"])
                    elif "bytes" in message and message["bytes"] is not None:
                        await backend_ws.send(message["bytes"])
            except (WebSocketDisconnect, RuntimeError):
                return

        async def backend_to_client() -> None:
            async for message in backend_ws:
                if isinstance(message, bytes):
                    await websocket.send_bytes(message)
                else:
                    await websocket.send_text(str(message))

        # Either side closing ends the relay; the other half is cancelled.
        tasks = [
            asyncio.create_task(client_to_backend()),
            asyncio.create_task(backend_to_client()),
        ]
        _done, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
        for task in pending:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
    except Exception:
        logger.warning("gateway proxy: websocket relay failed")
    finally:
        try:
            await backend_ws.close()
        except Exception:
            pass
        try:
            await websocket.close()
        except Exception:
            pass
