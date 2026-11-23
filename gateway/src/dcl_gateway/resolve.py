"""Matchmaker token → session locator. Fail closed; never a default session."""

from __future__ import annotations

from dataclasses import dataclass, field
from uuid import UUID
import base64
import logging

import httpx

logger = logging.getLogger(__name__)

_RESOLVE_PATH = "/v1/gateway/resolve"
_REDEEM_PATH = "/v1/gateway/redeem"


@dataclass(frozen=True)
class ResolvedSession:
    session_id: UUID
    target_url: str
    streamer: str
    # Login for the guest desktop. The gateway presents it; the renter never
    # sees it. None for a guest that asks for no password.
    upstream_user: str | None = None
    upstream_password: str | None = field(default=None, repr=False)

    def upstream_authorization(self) -> str | None:
        """`Authorization` header value for the guest, or None."""
        if not self.upstream_user or not self.upstream_password:
            return None
        raw = f"{self.upstream_user}:{self.upstream_password}".encode("utf-8")
        return "Basic " + base64.b64encode(raw).decode("ascii")


class ResolveError(Exception):
    """Lookup failed. Message must not include the grant token."""


async def _post(
    *,
    matchmaker_url: str,
    resolve_token: str,
    path: str,
    body: dict,
    timeout_seconds: float,
    transport: httpx.AsyncBaseTransport | None,
) -> dict | None:
    """POST to the Matchmaker's gateway API. 404/401/errors -> None."""
    headers = {
        "Authorization": f"Bearer {resolve_token}",
        "Accept": "application/json",
        "Content-Type": "application/json",
    }
    url = f"{matchmaker_url.rstrip('/')}{path}"
    try:
        async with httpx.AsyncClient(
            timeout=timeout_seconds,
            follow_redirects=False,
            transport=transport,
        ) as client:
            response = await client.post(url, json=body, headers=headers)
    except httpx.HTTPError:
        logger.warning("gateway resolve: matchmaker unreachable")
        return None

    if response.status_code == 404:
        return None
    if response.status_code != 200:
        logger.warning(
            "gateway resolve: matchmaker HTTP %s", response.status_code
        )
        return None
    try:
        parsed = response.json()
    except ValueError:
        return None
    return parsed if isinstance(parsed, dict) else None


def _session_from(body: dict) -> ResolvedSession | None:
    target = body.get("targetUrl")
    streamer = body.get("streamer")
    session_id = body.get("sessionId")
    if not isinstance(target, str) or not target.strip():
        return None
    if not isinstance(streamer, str) or not streamer.strip():
        return None
    if not isinstance(session_id, str) or not session_id.strip():
        return None
    try:
        parsed_id = UUID(session_id)
    except ValueError:
        return None
    user = body.get("upstreamUser")
    password = body.get("upstreamPassword")
    return ResolvedSession(
        session_id=parsed_id,
        target_url=target.strip(),
        streamer=streamer.strip(),
        upstream_user=user if isinstance(user, str) and user else None,
        upstream_password=password if isinstance(password, str) and password else None,
    )


async def resolve_token(
    *,
    matchmaker_url: str,
    resolve_token: str,
    grant_token: str,
    timeout_seconds: float = 5.0,
    transport: httpx.AsyncBaseTransport | None = None,
) -> ResolvedSession | None:
    """Session cookie -> locator. Fail closed."""
    body = await _post(
        matchmaker_url=matchmaker_url,
        resolve_token=resolve_token,
        path=_RESOLVE_PATH,
        body={"token": grant_token},
        timeout_seconds=timeout_seconds,
        transport=transport,
    )
    return _session_from(body) if body is not None else None


async def redeem_link(
    *,
    matchmaker_url: str,
    resolve_token: str,
    link_token: str,
    cookie_token: str | None = None,
    timeout_seconds: float = 5.0,
    transport: httpx.AsyncBaseTransport | None = None,
) -> tuple[ResolvedSession, str] | None:
    """Single-use desktop link -> (locator, session cookie). Fail closed.

    `cookie_token` is the cookie this browser already holds: a link it used
    before (second tab, double click) still opens for it, for no one else.
    """
    request: dict = {"token": link_token}
    if cookie_token:
        request["cookieToken"] = cookie_token
    body = await _post(
        matchmaker_url=matchmaker_url,
        resolve_token=resolve_token,
        path=_REDEEM_PATH,
        body=request,
        timeout_seconds=timeout_seconds,
        transport=transport,
    )
    if body is None:
        return None
    session = _session_from(body)
    cookie = body.get("cookieToken")
    if session is None or not isinstance(cookie, str) or not cookie:
        return None
    return session, cookie
