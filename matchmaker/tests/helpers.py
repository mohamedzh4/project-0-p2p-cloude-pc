from __future__ import annotations

from dataclasses import dataclass, field
from uuid import UUID

from fastapi.testclient import TestClient

from matchmaker.config import get_settings
from matchmaker.headscale import (
    HOST_ACL_TAGS,
    HeadscaleError,
    PreauthGrant,
)
from matchmaker.mesh import GuestMeshHint
from matchmaker.models import Host
from matchmaker.provision import STREAMER_KASM, STREAMER_SUNSHINE, ProvisionResult
from matchmaker.renterd import RenterdError
from matchmaker.storage import GuestStorageLocators

KASM_URL = "https://kasm.test.example/session"
KASM_TOKEN = "test-kasm-token"
PROVISION_URL = "https://provision.invalid/mock"
SUNSHINE_URL = "https://provision.invalid/sunshine"
MOONLIGHT_HOST = "sunshine.provision.invalid"


def host_auth_header() -> dict[str, str]:
    return {"Authorization": f"Bearer {get_settings().host_bootstrap_token}"}


def approve_auth_header() -> dict[str, str]:
    return {"Authorization": f"Bearer {get_settings().host_approve_token}"}


def inventory_payload(**overrides: object) -> dict:
    body: dict = {
        "displayName": "lab-pc",
        "labTier": "standard",
        "sharedLogicalCores": 4,
        "sharedMemoryBytes": 8 * 1024 * 1024 * 1024,
        "shareGpu": False,
        "sharedGpuId": None,
    }
    body.update(overrides)
    return body


def signup(
    client: TestClient,
    email: str = "user@example.com",
    password: str = "password123",
) -> dict:
    response = client.post("/v1/auth/signup", json={"email": email, "password": password})
    assert response.status_code == 201, response.text
    return response.json()


def auth_header(token: str) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


def heartbeat(client: TestClient, host_id: str):
    return client.post(f"/v1/hosts/{host_id}/heartbeat", headers=host_auth_header())


@dataclass
class FakeRenterd:
    """In-memory renterd S3 double. Swap via FastAPI Depends. No live Sia."""

    created: list[str] = field(default_factory=list)
    objects: list[tuple[str, str]] = field(default_factory=list)
    error: RenterdError | None = None
    existing_buckets: set[str] = field(default_factory=set)

    def create_bucket(self, name: str) -> bool:
        if self.error is not None:
            raise self.error
        if name in self.existing_buckets:
            return False
        self.existing_buckets.add(name)
        self.created.append(name)
        return True

    def put_object(self, bucket: str, key: str, body: bytes) -> None:
        if self.error is not None:
            raise self.error
        self.objects.append((bucket, key))

    def list_object_keys(self, bucket: str, prefix: str) -> list[str]:
        if self.error is not None:
            raise self.error
        return [key for stored_bucket, key in self.objects if stored_bucket == bucket and key.startswith(prefix)]


@dataclass
class FakeHeadscale:
    """In-memory Headscale double. Swap via FastAPI Depends. No live Headscale."""

    grant: PreauthGrant | None = None
    calls: list[str] = field(default_factory=list)
    error: HeadscaleError | None = None
    expired: list[str] = field(default_factory=list)
    tagged: list[tuple[tuple[str, ...], bool]] = field(default_factory=list)
    _serial: int = 0

    def mint_host_preauth(self) -> PreauthGrant:
        self.calls.append("mint")
        if self.error is not None:
            raise self.error
        if self.grant is not None:
            return self.grant
        return self.mint_tagged_preauth(acl_tags=HOST_ACL_TAGS, ephemeral=False)

    def mint_tagged_preauth(
        self, *, acl_tags: tuple[str, ...], ephemeral: bool
    ) -> PreauthGrant:
        self.calls.append("mint_tagged")
        self.tagged.append((acl_tags, ephemeral))
        if self.error is not None:
            raise self.error
        if self.grant is not None and acl_tags == HOST_ACL_TAGS and not ephemeral:
            return self.grant
        self._serial += 1
        tag = acl_tags[0].replace(":", "") if acl_tags else "none"
        return PreauthGrant(
            login_server="http://headscale.test.invalid",
            key=f"tskey-auth-test-{tag}-{self._serial}",
            user="1",
            reusable=False,
            ephemeral=ephemeral,
            expires_at="2026-09-05T12:00:00Z",
            acl_tags=acl_tags,
        )

    def expire_nodes_named(self, hostnames: tuple[str, ...]) -> int:
        self.calls.append("expire")
        if self.error is not None:
            raise self.error
        self.expired.extend(hostnames)
        return len(hostnames)


@dataclass
class FakeHostProvisioner:
    """Test double for the Host Agent provision port. Swap via FastAPI Depends."""

    connection_url: str
    connection_token: str | None = None
    sunshine_url: str = SUNSHINE_URL
    moonlight_host: str = MOONLIGHT_HOST
    calls: list[tuple[UUID, str]] = field(default_factory=list)
    storage_calls: list[GuestStorageLocators | None] = field(default_factory=list)
    mesh_calls: list[GuestMeshHint | None] = field(default_factory=list)

    def provision(
        self,
        host: Host,
        *,
        lab_tier: str,
        storage: GuestStorageLocators | None = None,
        guest_mesh: GuestMeshHint | None = None,
    ) -> ProvisionResult | None:
        self.calls.append((host.id, lab_tier))
        self.storage_calls.append(storage)
        self.mesh_calls.append(guest_mesh)
        if lab_tier == "pro":
            return ProvisionResult(
                connection_url=self.sunshine_url,
                connection_token=self.connection_token,
                streamer=STREAMER_SUNSHINE,
                sunshine_url=self.sunshine_url,
                moonlight_host=self.moonlight_host,
                moonlight_app="Desktop",
                gpu_exclusive=True,
            )
        return ProvisionResult(
            connection_url=self.connection_url,
            connection_token=self.connection_token,
            streamer=STREAMER_KASM,
        )
