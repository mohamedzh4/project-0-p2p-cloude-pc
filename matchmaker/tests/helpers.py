from __future__ import annotations

from dataclasses import dataclass, field
from uuid import UUID

from fastapi.testclient import TestClient

from matchmaker.config import get_settings
from matchmaker.models import Host
from matchmaker.provision import ProvisionResult
from matchmaker.renterd import RenterdError

KASM_URL = "https://kasm.test.example/session"
KASM_TOKEN = "test-kasm-token"
PROVISION_URL = "https://provision.invalid/mock"


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


@dataclass
class FakeHostProvisioner:
    """Test double for the Host Agent provision port. Swap via FastAPI Depends."""

    connection_url: str
    connection_token: str | None = None
    calls: list[tuple[UUID, str]] = field(default_factory=list)

    def provision(self, host: Host, *, lab_tier: str) -> ProvisionResult | None:
        self.calls.append((host.id, lab_tier))
        return ProvisionResult(
            connection_url=self.connection_url,
            connection_token=self.connection_token,
        )
