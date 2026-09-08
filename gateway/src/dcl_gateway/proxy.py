"""HTTP/WebSocket reverse proxy to a resolved session locator."""

from __future__ import annotations

from collections.abc import Mapping
from urllib.parse import urljoin, urlparse
import logging

import httpx
from fastapi import Request, WebSocket, WebSocketDisconnect
from fastapi.responses import Response

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


def join_backend(target_url: str, suffix: str, query: str) -> str:
    base = target_url if target_url.endswith("/") else f"{target_url}/"
    path = suffix.lstrip("/")
    joined = urljoin(base, path)
    if query:
        joiner = "&" if urlparse(joined).query else "?"
        return f"{joined}{joiner}{query}"
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


def _forward_request_headers(headers: Mapping[str, str], target_url: str) -> dict[str, str]:
    parsed = urlparse(target_url)
    forwarded: dict[str, str] = {}
    for key, value in headers.items():
        lowered = key.lower()
        if lowered in _HOP_BY_HOP:
            continue
        if lowered == "cookie":
            forwarded[key] = value
            continue
        forwarded[key] = value
    if parsed.netloc:
        forwarded["Host"] = parsed.netloc
    forwarded["X-Forwarded-Proto"] = "https"
    return forwarded


def _forward_response_headers(
    headers: Mapping[str, str],
    target_url: str,
    public_prefix: str,
) -> dict[str, str]:
    out: dict[str, str] = {}
    for key, value in headers.items():
        lowered = key.lower()
        if lowered in _HOP_BY_HOP:
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
) -> Response:
    backend = join_backend(target_url, suffix, request.url.query)
    headers = _forward_request_headers(request.headers, target_url)
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
        upstream.headers, target_url, public_prefix
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
) -> None:
    import websockets

    backend = join_backend(target_url, suffix, query)
    parsed = urlparse(backend)
    if parsed.scheme == "https":
        ws_url = backend.replace("https://", "wss://", 1)
    elif parsed.scheme == "http":
        ws_url = backend.replace("http://", "ws://", 1)
    else:
        await websocket.close(code=4400)
        return

    await websocket.accept()
    try:
        async with websockets.connect(
            ws_url,
            host=parsed.netloc,
            open_timeout=10,
        ) as backend_ws:
            async def client_to_backend() -> None:
                try:
                    while True:
                        message = await websocket.receive()
                        if message["type"] == "websocket.disconnect":
                            await backend_ws.close()
                            return
                        if "text" in message and message["text"] is not None:
                            await backend_ws.send(message["text"])
                        elif "bytes" in message and message["bytes"] is not None:
                            await backend_ws.send(message["bytes"])
                except WebSocketDisconnect:
                    await backend_ws.close()

            async def backend_to_client() -> None:
                try:
                    async for message in backend_ws:
                        if isinstance(message, bytes):
                            await websocket.send_bytes(message)
                        else:
                            await websocket.send_text(str(message))
                except Exception:
                    await websocket.close()

            import asyncio

            await asyncio.gather(client_to_backend(), backend_to_client())
    except Exception:
        logger.warning("gateway proxy: websocket backend unreachable")
        try:
            await websocket.close(code=1014)
        except Exception:
            pass
