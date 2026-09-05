"""Headscale HTTP client — mint a Host Agent preauth key.

Architecture §7: the agent asks the Matchmaker for a pre-auth key and joins
the mesh. This module talks to a self-hosted Headscale instance (no Tailscale
SaaS).

Phase 14c: mint tags the Host Agent node `tag:host` so the checked-in
default-deny policy (`headscale/acl.hujson`) can distinguish volunteers from
guests. That is **host-tagged**, not a per-session identity. Guest/user tags
are not minted here. Volunteer hosts must not become exit nodes — see
`docs/runbooks/headscale.md`.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Protocol

import httpx

_PREAUTH_PATH = "/api/v1/preauthkey"
_REQUEST_TIMEOUT_SECONDS = 10.0
_PREAUTH_TTL = timedelta(hours=1)

# Applied on every Host Agent preauth. Not ephemeral, not per-session.
HOST_ACL_TAG = "tag:host"
HOST_ACL_TAGS = (HOST_ACL_TAG,)


class HeadscaleError(Exception):
    """Headscale mint failed. Message must not include API or preauth keys."""


@dataclass(frozen=True)
class PreauthGrant:
    """A one-shot join grant. `key` is a secret — do not log it."""

    login_server: str
    key: str
    user: str
    reusable: bool
    ephemeral: bool
    expires_at: str
    acl_tags: tuple[str, ...]


class HeadscaleClient(Protocol):
    def mint_host_preauth(self) -> PreauthGrant:
        """Create a single-use preauth key for a Host Agent node (`tag:host`)."""


class HttpHeadscaleClient:
    """POST `{HEADSCALE_URL}/api/v1/preauthkey` with the Headscale API Bearer.

    Redirects are refused so the API key is not replayed against another host.
    """

    def __init__(
        self,
        *,
        base_url: str,
        api_key: str,
        user: str,
        login_server: str,
        timeout_seconds: float = _REQUEST_TIMEOUT_SECONDS,
        transport: httpx.BaseTransport | None = None,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key
        self._user = user
        self._login_server = login_server.rstrip("/")
        self._timeout_seconds = timeout_seconds
        self._transport = transport

    def mint_host_preauth(self) -> PreauthGrant:
        expiration = (datetime.now(timezone.utc) + _PREAUTH_TTL).strftime(
            "%Y-%m-%dT%H:%M:%SZ"
        )
        tags = list(HOST_ACL_TAGS)
        payload = {
            "user": self._user,
            "reusable": False,
            "ephemeral": False,
            "expiration": expiration,
            # Headscale 0.26 proto JSON uses acl_tags; later HTTP uses aclTags.
            "aclTags": tags,
            "acl_tags": tags,
        }
        headers = {
            "Authorization": f"Bearer {self._api_key}",
            "Accept": "application/json",
            "Content-Type": "application/json",
        }
        url = f"{self._base_url}{_PREAUTH_PATH}"
        try:
            with httpx.Client(
                timeout=self._timeout_seconds,
                follow_redirects=False,
                transport=self._transport,
            ) as client:
                response = client.post(url, json=payload, headers=headers)
        except httpx.HTTPError as exc:
            raise HeadscaleError("could not reach Headscale") from exc

        if response.status_code != 200:
            raise HeadscaleError(f"Headscale rejected preauth mint (HTTP {response.status_code})")

        try:
            body = response.json()
        except ValueError as exc:
            raise HeadscaleError("Headscale returned a non-JSON preauth response") from exc
        if not isinstance(body, dict):
            raise HeadscaleError("Headscale returned a non-object preauth response")

        key = _extract_preauth_key(body)
        return PreauthGrant(
            login_server=self._login_server,
            key=key,
            user=self._user,
            reusable=False,
            ephemeral=False,
            expires_at=expiration,
            acl_tags=HOST_ACL_TAGS,
        )


def _extract_preauth_key(body: dict) -> str:
    for wrapper in ("preAuthKey", "pre_auth_key", "preauthKey"):
        node = body.get(wrapper)
        if isinstance(node, dict):
            raw = node.get("key")
            if isinstance(raw, str) and raw.strip():
                return raw.strip()
    raw = body.get("key")
    if isinstance(raw, str) and raw.strip():
        return raw.strip()
    raise HeadscaleError("Headscale preauth response did not include a key")
