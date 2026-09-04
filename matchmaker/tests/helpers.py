from __future__ import annotations

from fastapi.testclient import TestClient

from matchmaker.config import get_settings

KASM_URL = "https://kasm.test.example/session"
KASM_TOKEN = "test-kasm-token"


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
