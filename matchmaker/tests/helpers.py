from __future__ import annotations

from fastapi.testclient import TestClient

KASM_URL = "https://kasm.test.example/session"
KASM_TOKEN = "test-kasm-token"


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
