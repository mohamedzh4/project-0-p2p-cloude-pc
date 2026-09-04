"""Host Agent provision port. OS/backend selection stays on the agent.

The session router calls [`HostProvisioner`] after `HostSelector.select`. It
must not import a concrete selector, and it must not name hypervisor backends.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol
from urllib.parse import urlparse

import httpx
from sqlalchemy.orm import Session

from matchmaker.models import Host

_PROVISION_PATH = "/v1/provision"
_REQUEST_TIMEOUT_SECONDS = 10.0


@dataclass(frozen=True)
class ProvisionResult:
    """Connection returned by a ready (or stub) agent. Session JSON stays kasmUrl."""

    connection_url: str
    connection_token: str | None = None


class HostProvisioner(Protocol):
    def provision(self, host: Host, *, lab_tier: str) -> ProvisionResult | None:
        """Ask the selected host's agent to become ready, or None to use the host row."""


def apply_provision_result(db: Session, host: Host, result: ProvisionResult) -> None:
    """Keep existing-session 200 on the same connection URL without a new field."""
    host.kasm_url = result.connection_url
    if result.connection_token is not None:
        host.kasm_token = result.connection_token
    db.commit()
    db.refresh(host)


def provision_booked_host(
    db: Session,
    host: Host,
    provisioner: HostProvisioner,
    *,
    lab_tier: str,
) -> None:
    result = provisioner.provision(host, lab_tier=lab_tier)
    if result is None or not result.connection_url.strip():
        return
    apply_provision_result(db, host, result)


def agent_origin(value: str | None) -> str | None:
    """Return an http(s) origin, or None if missing/unusable. No redirects later."""
    if value is None:
        return None
    stripped = value.strip().rstrip("/")
    if not stripped:
        return None
    parsed = urlparse(stripped)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        return None
    if parsed.username or parsed.password:
        return None
    return stripped


class HttpHostProvisioner:
    """POST `{origin}/v1/provision` with the host bootstrap Bearer.

    Origin is `host.agent_url`, else the optional Matchmaker fallback URL.
    Network/HTTP failures return None so booking can use the seeded kasm URL.
    """

    def __init__(
        self,
        *,
        fallback_base_url: str | None = None,
        bearer_token: str,
        timeout_seconds: float = _REQUEST_TIMEOUT_SECONDS,
    ) -> None:
        self._fallback_base_url = fallback_base_url
        self._bearer_token = bearer_token
        self._timeout_seconds = timeout_seconds

    def provision(self, host: Host, *, lab_tier: str) -> ProvisionResult | None:
        origin = agent_origin(host.agent_url) or agent_origin(self._fallback_base_url)
        if origin is None:
            return None

        url = f"{origin}{_PROVISION_PATH}"
        payload = {
            "labTier": lab_tier,
            "sharedGpuId": host.shared_gpu_id,
        }
        headers = {
            "Authorization": f"Bearer {self._bearer_token}",
            "Accept": "application/json",
            "Content-Type": "application/json",
        }
        try:
            with httpx.Client(timeout=self._timeout_seconds, follow_redirects=False) as client:
                response = client.post(url, json=payload, headers=headers)
        except httpx.HTTPError:
            return None

        if response.status_code != 200:
            return None

        try:
            body = response.json()
        except ValueError:
            return None
        if not isinstance(body, dict):
            return None

        connection_url = body.get("connectionUrl")
        if not isinstance(connection_url, str) or not connection_url.strip():
            return None
        if body.get("hypervisorInvoked") is True:
            # Stubs never invoke a hypervisor. Refuse a live boot claimed here.
            return None

        token = body.get("connectionToken")
        connection_token = token.strip() if isinstance(token, str) and token.strip() else None
        return ProvisionResult(
            connection_url=connection_url.strip(),
            connection_token=connection_token,
        )
