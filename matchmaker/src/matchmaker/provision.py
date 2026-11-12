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

from matchmaker.images import ImageManifest
from matchmaker.mesh import GuestMeshHint
from matchmaker.models import Host, LabTier
from matchmaker.schemas import ConnectionInfo, MoonlightInfo
from matchmaker.storage import GuestStorageLocators

_PROVISION_PATH = "/v1/provision"
_TEARDOWN_PATH = "/v1/teardown"
# Host Agent may poll a cold Kasm/Sunshine guest for up to five minutes.
_REQUEST_TIMEOUT_SECONDS = 330.0
_TEARDOWN_TIMEOUT_SECONDS = 15.0

STREAMER_KASM = "kasm"
STREAMER_SUNSHINE = "sunshine"
_MOONLIGHT_APP = "Desktop"


@dataclass(frozen=True)
class ProvisionResult:
    """Connection returned by a ready (or stub) agent.

    Standard session JSON uses kasmUrl. Pro uses sunshineUrl / Moonlight —
    not a renamed Kasm field. The host row still stores the locator in
    `kasm_url` so existing-session 200 needs no extra column.
    """

    connection_url: str
    connection_token: str | None = None
    streamer: str = STREAMER_KASM
    sunshine_url: str | None = None
    moonlight_host: str | None = None
    moonlight_app: str | None = None
    gpu_exclusive: bool = False

    def locator(self) -> str:
        if self.streamer == STREAMER_SUNSHINE:
            url = self.sunshine_url or self.connection_url
            return url.strip()
        return self.connection_url


class HostProvisioner(Protocol):
    def provision(
        self,
        host: Host,
        *,
        lab_tier: str,
        storage: GuestStorageLocators | None = None,
        guest_mesh: GuestMeshHint | None = None,
    ) -> ProvisionResult | None:
        """Ask the selected host's agent to become ready, or None to use the host row."""

    def teardown(self, host: Host) -> None:
        """Best-effort POST /v1/teardown. Occupancy is already released in the DB."""


def locator_is_placeholder(url: str | None) -> bool:
    """RFC 2606 `.invalid` (seed example.invalid and stub provision.invalid)."""
    if url is None or not url.strip():
        return True
    host = (urlparse(url.strip()).hostname or "").lower().rstrip(".")
    return host == "invalid" or host.endswith(".invalid")


def apply_provision_result(db: Session, host: Host, result: ProvisionResult) -> None:
    """Keep existing-session 200 on the same locator without a new column."""
    host.kasm_url = result.locator()
    if result.connection_token is not None:
        host.kasm_token = result.connection_token
    db.commit()
    db.refresh(host)


def session_connection(host: Host) -> ConnectionInfo:
    """Map the host row to Standard Kasm vs Pro Sunshine. GATE-PRO stays open."""
    if host.lab_tier == LabTier.PRO:
        sunshine_url = host.kasm_url
        return ConnectionInfo(
            streamer=STREAMER_SUNSHINE,
            kasm_url=None,
            kasm_token=None,
            sunshine_url=sunshine_url,
            moonlight=MoonlightInfo(
                host=_hostname(sunshine_url),
                app=_MOONLIGHT_APP,
            ),
            gpu_exclusive=True,
        )
    return ConnectionInfo(
        streamer=STREAMER_KASM,
        kasm_url=host.kasm_url,
        kasm_token=host.kasm_token,
        sunshine_url=None,
        moonlight=None,
        gpu_exclusive=False,
    )


def _hostname(url: str) -> str:
    host = urlparse(url).hostname
    return host if host else url


def _optional_str(value: object) -> str | None:
    if isinstance(value, str) and value.strip():
        return value.strip()
    return None


def provision_booked_host(
    db: Session,
    host: Host,
    provisioner: HostProvisioner,
    *,
    lab_tier: str,
    storage: GuestStorageLocators | None = None,
    guest_mesh: GuestMeshHint | None = None,
) -> bool:
    """Overwrite the host locator with a live agent URL.

    Returns False when the agent is down, returns a placeholder, or yields
    nothing while the host row is still `.invalid`. Callers must not occupy
    a host with a placeholder. A live URL already on the host row is kept
    only when the agent did not answer.
    """
    result = provisioner.provision(
        host,
        lab_tier=lab_tier,
        storage=storage,
        guest_mesh=guest_mesh,
    )
    if result is None or not result.connection_url.strip():
        return not locator_is_placeholder(host.kasm_url)
    if locator_is_placeholder(result.locator()):
        return False
    apply_provision_result(db, host, result)
    return True


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
    Network/HTTP failures return None. Placeholder (`.invalid`) locators are
    never treated as success — the session router then fails the book instead
    of occupying the host with a seed URL.
    """

    def __init__(
        self,
        *,
        fallback_base_url: str | None = None,
        bearer_token: str,
        timeout_seconds: float = _REQUEST_TIMEOUT_SECONDS,
        images: dict[str, ImageManifest] | None = None,
        storage_endpoint: str | None = None,
    ) -> None:
        self._fallback_base_url = fallback_base_url
        self._bearer_token = bearer_token
        self._timeout_seconds = timeout_seconds
        self._images = images or {}
        self._storage_endpoint = storage_endpoint

    def provision(
        self,
        host: Host,
        *,
        lab_tier: str,
        storage: GuestStorageLocators | None = None,
        guest_mesh: GuestMeshHint | None = None,
    ) -> ProvisionResult | None:
        origin = agent_origin(host.agent_url) or agent_origin(self._fallback_base_url)
        if origin is None:
            return None

        url = f"{origin}{_PROVISION_PATH}"
        payload: dict[str, object] = {
            "labTier": lab_tier,
            "sharedGpuId": host.shared_gpu_id,
        }
        manifest = self._images.get(lab_tier)
        if manifest is not None:
            payload["imageDigest"] = manifest.digest_sha256
            payload["imageSignature"] = manifest.signature
        # Locators only — never S3 secrets. Host Agent must not mount this on
        # the volunteer OS; rclone belongs inside the guest.
        if storage is not None:
            payload["storageEndpoint"] = storage.endpoint
            payload["storageBucket"] = storage.bucket
            payload["storagePrefix"] = storage.prefix
            payload["storageRegion"] = storage.region
        elif self._storage_endpoint:
            payload["storageEndpoint"] = self._storage_endpoint
        if guest_mesh is not None:
            mesh: dict[str, object] = {
                "loginServer": guest_mesh.login_server,
                "hostname": guest_mesh.hostname,
                "exitNode": guest_mesh.exit_node,
                "aclTags": list(guest_mesh.acl_tags),
            }
            if guest_mesh.bootstrap_origin:
                mesh["bootstrapOrigin"] = guest_mesh.bootstrap_origin
            if guest_mesh.bootstrap_token:
                mesh["bootstrapToken"] = guest_mesh.bootstrap_token
            payload["guestMesh"] = mesh
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
        if locator_is_placeholder(connection_url):
            return None

        token = body.get("connectionToken")
        connection_token = token.strip() if isinstance(token, str) and token.strip() else None
        streamer = STREAMER_SUNSHINE if lab_tier == LabTier.PRO else STREAMER_KASM
        raw_streamer = _optional_str(body.get("streamer"))
        if raw_streamer in {STREAMER_KASM, STREAMER_SUNSHINE}:
            streamer = raw_streamer
        sunshine_url = _optional_str(body.get("sunshineUrl"))
        moonlight_host: str | None = None
        moonlight_app: str | None = None
        moonlight = body.get("moonlight")
        if isinstance(moonlight, dict):
            moonlight_host = _optional_str(moonlight.get("host"))
            moonlight_app = _optional_str(moonlight.get("app")) or _MOONLIGHT_APP
        if streamer == STREAMER_SUNSHINE and sunshine_url is None:
            sunshine_url = connection_url.strip()
        gpu_exclusive = body.get("gpuExclusive") is True or lab_tier == LabTier.PRO
        return ProvisionResult(
            connection_url=connection_url.strip(),
            connection_token=connection_token,
            streamer=streamer,
            sunshine_url=sunshine_url,
            moonlight_host=moonlight_host,
            moonlight_app=moonlight_app,
            gpu_exclusive=gpu_exclusive,
        )

    def teardown(self, host: Host) -> None:
        origin = agent_origin(host.agent_url) or agent_origin(self._fallback_base_url)
        if origin is None:
            return
        url = f"{origin}{_TEARDOWN_PATH}"
        headers = {
            "Authorization": f"Bearer {self._bearer_token}",
            "Accept": "application/json",
        }
        try:
            with httpx.Client(
                timeout=min(self._timeout_seconds, _TEARDOWN_TIMEOUT_SECONDS),
                follow_redirects=False,
            ) as client:
                client.post(url, headers=headers)
        except httpx.HTTPError:
            return


def teardown_booked_host(provisioner: HostProvisioner, host: Host | None) -> None:
    """Ignore agent failures — Matchmaker occupancy is already released."""
    if host is None:
        return
    try:
        provisioner.teardown(host)
    except Exception:
        return
