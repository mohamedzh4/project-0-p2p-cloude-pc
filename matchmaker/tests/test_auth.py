from __future__ import annotations

from fastapi.testclient import TestClient

from tests.helpers import auth_header, signup


def test_signup_returns_jwt_and_user(client: TestClient) -> None:
    data = signup(client, "ada@example.com")
    assert data["tokenType"] == "bearer"
    assert data["accessToken"]
    assert data["user"]["email"] == "ada@example.com"
    assert data["user"]["id"]


def test_signup_lowercases_email(client: TestClient) -> None:
    data = signup(client, "Ada@Example.COM")
    assert data["user"]["email"] == "ada@example.com"


def test_duplicate_email_is_conflict(client: TestClient) -> None:
    signup(client, "ada@example.com")
    response = client.post(
        "/v1/auth/signup",
        json={"email": "ADA@example.com", "password": "password123"},
    )
    assert response.status_code == 409


def test_short_password_is_rejected(client: TestClient) -> None:
    response = client.post(
        "/v1/auth/signup",
        json={"email": "ada@example.com", "password": "short"},
    )
    assert response.status_code == 422


def test_login_returns_jwt(client: TestClient) -> None:
    signup(client, "ada@example.com", "password123")
    response = client.post(
        "/v1/auth/login",
        json={"email": "ADA@example.com", "password": "password123"},
    )
    assert response.status_code == 200
    data = response.json()
    assert data["accessToken"]
    assert data["user"]["email"] == "ada@example.com"


def test_login_wrong_password_is_unauthorized(client: TestClient) -> None:
    signup(client, "ada@example.com", "password123")
    response = client.post(
        "/v1/auth/login",
        json={"email": "ada@example.com", "password": "wrong-password"},
    )
    assert response.status_code == 401


def test_login_unknown_email_is_unauthorized(client: TestClient) -> None:
    response = client.post(
        "/v1/auth/login",
        json={"email": "missing@example.com", "password": "password123"},
    )
    assert response.status_code == 401


def test_me_requires_auth(client: TestClient) -> None:
    response = client.get("/v1/auth/me")
    assert response.status_code == 401


def test_me_rejects_invalid_token(client: TestClient) -> None:
    response = client.get("/v1/auth/me", headers=auth_header("not-a-jwt"))
    assert response.status_code == 401


def test_me_returns_the_logged_in_user(client: TestClient) -> None:
    data = signup(client, "ada@example.com")
    response = client.get("/v1/auth/me", headers=auth_header(data["accessToken"]))
    assert response.status_code == 200
    body = response.json()
    assert body["email"] == "ada@example.com"
    assert body["id"] == data["user"]["id"]
