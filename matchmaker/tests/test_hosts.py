from __future__ import annotations

from uuid import uuid4

from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.models import Host, HostStatus, LabSession
from matchmaker.seed import seed_host
from tests.helpers import (
    KASM_URL,
    approve_auth_header,
    auth_header,
    host_auth_header,
    inventory_payload,
    signup,
)


def test_register_host_starts_pending(client: TestClient, db: Session) -> None:
    response = client.post("/v1/hosts", headers=host_auth_header(), json=inventory_payload())
    assert response.status_code == 201
    data = response.json()
    assert data["status"] == "pending"
    assert data["sharedLogicalCores"] == 4
    assert data["shareGpu"] is False
    assert data["sharedGpuId"] is None
    assert data["kasmUrl"] is None
    stored = db.get(Host, data["hostId"])
    assert stored is not None
    assert stored.status == HostStatus.PENDING


def test_register_requires_host_bootstrap_token(client: TestClient) -> None:
    missing = client.post("/v1/hosts", json=inventory_payload())
    assert missing.status_code == 401

    user_token = signup(client)["accessToken"]
    as_user = client.post(
        "/v1/hosts",
        headers=auth_header(user_token),
        json=inventory_payload(),
    )
    assert as_user.status_code == 401

    as_approver = client.post(
        "/v1/hosts",
        headers=approve_auth_header(),
        json=inventory_payload(),
    )
    assert as_approver.status_code == 401


def test_gpu_id_round_trips_on_register_and_update(client: TestClient) -> None:
    created = client.post(
        "/v1/hosts",
        headers=host_auth_header(),
        json=inventory_payload(
            shareGpu=True,
            sharedGpuId="GPU-TEST-4070",
            kasmUrl="https://kasm.test.example/pending",
        ),
    )
    assert created.status_code == 201
    body = created.json()
    assert body["shareGpu"] is True
    assert body["sharedGpuId"] == "GPU-TEST-4070"
    assert body["kasmUrl"] == "https://kasm.test.example/pending"
    host_id = body["hostId"]

    updated = client.put(
        f"/v1/hosts/{host_id}",
        headers=host_auth_header(),
        json=inventory_payload(
            displayName="lab-pc",
            shareGpu=True,
            sharedGpuId="nvidia:1",
        ),
    )
    assert updated.status_code == 200
    assert updated.json()["status"] == "pending"
    assert updated.json()["sharedGpuId"] == "nvidia:1"
    assert updated.json()["kasmUrl"] == "https://kasm.test.example/pending"


def test_gpu_id_is_kept_when_sharing_is_off(client: TestClient) -> None:
    response = client.post(
        "/v1/hosts",
        headers=host_auth_header(),
        json=inventory_payload(shareGpu=False, sharedGpuId="GPU-KEPT-OFF"),
    )
    assert response.status_code == 201
    assert response.json()["shareGpu"] is False
    assert response.json()["sharedGpuId"] == "GPU-KEPT-OFF"


def test_share_gpu_without_id_is_rejected(client: TestClient) -> None:
    response = client.post(
        "/v1/hosts",
        headers=host_auth_header(),
        json=inventory_payload(shareGpu=True, sharedGpuId=None),
    )
    assert response.status_code == 422
    assert "exclusive" in response.text
    assert "concurrent slice" in response.text


def test_update_does_not_auto_approve(client: TestClient) -> None:
    created = client.post("/v1/hosts", headers=host_auth_header(), json=inventory_payload())
    host_id = created.json()["hostId"]
    updated = client.put(
        f"/v1/hosts/{host_id}",
        headers=host_auth_header(),
        json=inventory_payload(sharedLogicalCores=8),
    )
    assert updated.status_code == 200
    assert updated.json()["status"] == "pending"
    assert updated.json()["sharedLogicalCores"] == 8


def test_put_creates_pending_host_when_missing(client: TestClient) -> None:
    host_id = str(uuid4())
    response = client.put(
        f"/v1/hosts/{host_id}",
        headers=host_auth_header(),
        json=inventory_payload(displayName="stable-id-pc"),
    )
    assert response.status_code == 201
    assert response.json()["hostId"] == host_id
    assert response.json()["status"] == "pending"


def test_pending_host_is_not_booked(
    client: TestClient, pending_host: Host, db: Session
) -> None:
    token = signup(client)["accessToken"]
    response = client.post("/v1/sessions", headers=auth_header(token))
    assert response.status_code == 409
    assert db.scalars(select(LabSession)).first() is None
    assert pending_host.status == HostStatus.PENDING


def test_approved_host_is_booked_not_pending(
    client: TestClient, seeded_host: Host, pending_host: Host
) -> None:
    token = signup(client)["accessToken"]
    response = client.post("/v1/sessions", headers=auth_header(token))
    assert response.status_code == 201
    assert response.json()["hostId"] == str(seeded_host.id)
    assert response.json()["hostId"] != str(pending_host.id)


def test_approve_then_host_can_be_booked(client: TestClient) -> None:
    created = client.post(
        "/v1/hosts",
        headers=host_auth_header(),
        json=inventory_payload(kasmUrl=KASM_URL),
    )
    host_id = created.json()["hostId"]

    token = signup(client)["accessToken"]
    blocked = client.post("/v1/sessions", headers=auth_header(token))
    assert blocked.status_code == 409

    bootstrap_approve = client.post(
        f"/v1/hosts/{host_id}/approve",
        headers=host_auth_header(),
    )
    assert bootstrap_approve.status_code == 401

    approved = client.post(
        f"/v1/hosts/{host_id}/approve",
        headers=approve_auth_header(),
    )
    assert approved.status_code == 200
    assert approved.json()["status"] == "approved"
    assert approved.json()["hostId"] == host_id

    booked = client.post("/v1/sessions", headers=auth_header(token))
    assert booked.status_code == 201
    assert booked.json()["hostId"] == host_id


def test_approve_unknown_host_is_not_found(client: TestClient) -> None:
    response = client.post(
        f"/v1/hosts/{uuid4()}/approve",
        headers=approve_auth_header(),
    )
    assert response.status_code == 404


def test_seed_upserts_by_display_name_and_leaves_other_hosts(
    db: Session, pending_host: Host, monkeypatch
) -> None:
    monkeypatch.setenv("SEED_HOST_DISPLAY_NAME", "phase1-lab-1")
    monkeypatch.setenv("SEED_HOST_KASM_URL", "https://seed.test.example/kasm")
    monkeypatch.setenv("SEED_HOST_SHARE_GPU", "true")
    monkeypatch.setenv("SEED_HOST_SHARED_GPU_ID", "GPU-SEED")

    first = seed_host()
    assert first.status == HostStatus.APPROVED
    assert first.shared_gpu_id == "GPU-SEED"
    assert first.display_name == "phase1-lab-1"

    db.expire_all()
    other = db.get(Host, pending_host.id)
    assert other is not None
    assert other.status == HostStatus.PENDING

    second = seed_host()
    assert second.id == first.id
    db.expire_all()
    assert db.get(Host, pending_host.id) is not None
