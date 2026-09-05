"""Headscale HTTP client — mint preauth keys and expire session nodes.

Architecture §7: the agent asks the Matchmaker for a pre-auth key and joins
the mesh. This module talks to a self-hosted Headscale instance (no Tailscale
SaaS).

Host Agent mint stays `tag:host` (non-ephemeral). Session book mints
ephemeral `tag:guest` and `tag:user` keys (Headscale 0.26 `/api/v1/preauthkey`
`aclTags`). Volunteer hosts must not become exit nodes — see
`docs/runbooks/headscale.md`.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Protocol

import httpx

_PREAUTH_PATH = "/api/v1/preauthkey"
_NODES_PATH = "/api/v1/node"
_REQUEST_TIMEOUT_SECONDS = 10.0
_HOST_PREAUTH_TTL = timedelta(hours=1)
_SESSION_PREAUTH_TTL = timedelta(hours=4)

HOST_ACL_TAG = "tag:host"
GUEST_ACL_TAG = "tag:guest"
USER_ACL_TAG = "tag:user"
HOST_ACL_TAGS = (HOST_ACL_TAG,)
GUEST_ACL_TAGS = (GUEST_ACL_TAG,)
USER_ACL_TAGS = (USER_ACL_TAG,)
DEFAULT_EXIT_NODE = "dcl-platform-exit"


class HeadscaleError(Exception):
    """Headscale call failed. Message must not include API or preauth keys."""


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

    def mint_tagged_preauth(
        self, *, acl_tags: tuple[str, ...], ephemeral: bool
    ) -> PreauthGrant:
        """Create a preauth key with Headscale ACL tags (0.26 `aclTags`)."""

    def expire_nodes_named(self, hostnames: tuple[str, ...]) -> int:
        """Expire nodes whose hostname matches. Returns how many expire calls succeeded."""


class HttpHeadscaleClient:
    """Headscale 0.26 REST (`/api/v1/preauthkey`, `/api/v1/node`).

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
        return self.mint_tagged_preauth(acl_tags=HOST_ACL_TAGS, ephemeral=False)

    def mint_tagged_preauth(
        self, *, acl_tags: tuple[str, ...], ephemeral: bool
    ) -> PreauthGrant:
        if not acl_tags:
            raise HeadscaleError("Headscale mint requires at least one ACL tag")
        ttl = _SESSION_PREAUTH_TTL if ephemeral else _HOST_PREAUTH_TTL
        expiration = (datetime.now(timezone.utc) + ttl).strftime("%Y-%m-%dT%H:%M:%SZ")
        tags = list(acl_tags)
        payload = {
            "user": self._user,
            "reusable": False,
            "ephemeral": ephemeral,
            "expiration": expiration,
            # Headscale 0.26 proto JSON uses acl_tags; HTTP uses aclTags.
            "aclTags": tags,
            "acl_tags": tags,
        }
        body = self._request_json("POST", _PREAUTH_PATH, json_body=payload)
        key = _extract_preauth_key(body)
        return PreauthGrant(
            login_server=self._login_server,
            key=key,
            user=self._user,
            reusable=False,
            ephemeral=ephemeral,
            expires_at=expiration,
            acl_tags=acl_tags,
        )

    def expire_nodes_named(self, hostnames: tuple[str, ...]) -> int:
        wanted = {name.strip() for name in hostnames if name and name.strip()}
        if not wanted:
            return 0
        listed = self._request_json("GET", _NODES_PATH)
        expired = 0
        for node in _extract_nodes(listed):
            if _node_hostname(node) not in wanted:
                continue
            node_id = _node_id(node)
            if node_id is None:
                continue
            self._expire_node(node_id)
            expired += 1
        return expired

    def _expire_node(self, node_id: str) -> None:
        self._request_json("POST", f"{_NODES_PATH}/{node_id}/expire", json_body={})

    def _request_json(
        self,
        method: str,
        path: str,
        *,
        json_body: dict | None = None,
    ) -> dict:
        headers = {
            "Authorization": f"Bearer {self._api_key}",
            "Accept": "application/json",
            "Content-Type": "application/json",
        }
        url = f"{self._base_url}{path}"
        try:
            with httpx.Client(
                timeout=self._timeout_seconds,
                follow_redirects=False,
                transport=self._transport,
            ) as client:
                response = client.request(method, url, json=json_body, headers=headers)
        except httpx.HTTPError as exc:
            raise HeadscaleError("could not reach Headscale") from exc

        if response.status_code not in {200, 201}:
            raise HeadscaleError(
                f"Headscale rejected {method} {path} (HTTP {response.status_code})"
            )

        if not response.content:
            return {}
        try:
            body = response.json()
        except ValueError as exc:
            raise HeadscaleError("Headscale returned a non-JSON response") from exc
        if isinstance(body, dict):
            return body
        if isinstance(body, list):
            return {"nodes": body}
        raise HeadscaleError("Headscale returned a non-object response")


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


def _extract_nodes(body: dict) -> list[dict]:
    for key in ("nodes", "Nodes"):
        raw = body.get(key)
        if isinstance(raw, list):
            return [item for item in raw if isinstance(item, dict)]
    return []


def _node_id(node: dict) -> str | None:
    for key in ("id", "nodeId", "node_id"):
        raw = node.get(key)
        if isinstance(raw, int):
            return str(raw)
        if isinstance(raw, str) and raw.strip():
            return raw.strip()
    return None


def _node_hostname(node: dict) -> str:
    for key in ("givenName", "given_name", "name", "hostname"):
        raw = node.get(key)
        if isinstance(raw, str) and raw.strip():
            return raw.strip()
    return ""
