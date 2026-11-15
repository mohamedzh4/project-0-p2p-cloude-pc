from __future__ import annotations

from pathlib import Path
from uuid import UUID

import httpx
from fastapi.testclient import TestClient

from matchmaker.renterd import HttpRenterdClient, RenterdError
from matchmaker.storage import (
    PROVISION_MARKER_KEY,
    bucket_name_for_user,
    guest_rclone_args,
    guest_rclone_mount_command,
)
from tests.helpers import FakeRenterd, auth_header, signup

TEST_ACCESS = "test-renterd-access"
TEST_SECRET = "test-renterd-secret-key"


def test_http_client_create_bucket_and_put_object_against_mocked_s3() -> None:
    recorded: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        recorded.append(request)
        if request.method == "PUT" and request.url.path == "/dcl-test-bucket":
            return httpx.Response(200)
        if request.method == "PUT" and "/.dcl/provisioned" in request.url.path:
            return httpx.Response(200)
        return httpx.Response(500, text="unexpected")

    client = HttpRenterdClient(
        endpoint="http://renterd.test.invalid",
        access_key=TEST_ACCESS,
        secret_key=TEST_SECRET,
        region="us-east-1",
        transport=httpx.MockTransport(handler),
    )
    created = client.create_bucket("dcl-test-bucket")
    client.put_object("dcl-test-bucket", PROVISION_MARKER_KEY, b"ok\n")

    assert created is True
    assert len(recorded) == 2
    create_req, put_req = recorded
    assert create_req.method == "PUT"
    assert create_req.url.path == "/dcl-test-bucket"
    assert create_req.headers["Authorization"].startswith("AWS4-HMAC-SHA256 ")
    assert TEST_SECRET not in create_req.headers["Authorization"]
    assert put_req.url.path == f"/dcl-test-bucket/{PROVISION_MARKER_KEY}"
    assert put_req.content == b"ok\n"


def test_http_client_lists_prefix_against_mocked_s3() -> None:
    recorded: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        recorded.append(request)
        if request.method == "GET" and request.url.params.get("list-type") == "2":
            xml = (
                '<?xml version="1.0"?>'
                "<ListBucketResult>"
                "<Contents><Key>files/.dcl/provisioned</Key></Contents>"
                "</ListBucketResult>"
            )
            return httpx.Response(200, content=xml.encode("utf-8"))
        return httpx.Response(500, text="unexpected")

    client = HttpRenterdClient(
        endpoint="http://renterd.test.invalid",
        access_key=TEST_ACCESS,
        secret_key=TEST_SECRET,
        region="us-east-1",
        transport=httpx.MockTransport(handler),
    )
    keys = client.list_object_keys("dcl-test-bucket", "files/")
    assert keys == ["files/.dcl/provisioned"]
    assert len(recorded) == 1
    assert recorded[0].method == "GET"
    assert recorded[0].url.path == "/dcl-test-bucket"
    assert recorded[0].url.params["prefix"] == "files/"
    assert recorded[0].headers["Authorization"].startswith("AWS4-HMAC-SHA256 ")
    assert TEST_SECRET not in recorded[0].headers["Authorization"]


def test_http_client_treats_409_as_existing_bucket() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(409)

    client = HttpRenterdClient(
        endpoint="http://renterd.test.invalid",
        access_key=TEST_ACCESS,
        secret_key=TEST_SECRET,
        region="us-east-1",
        transport=httpx.MockTransport(handler),
    )
    assert client.create_bucket("dcl-already") is False


def test_http_client_does_not_follow_redirects() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(302, headers={"Location": "https://evil.example/steal"})

    client = HttpRenterdClient(
        endpoint="http://renterd.test.invalid",
        access_key=TEST_ACCESS,
        secret_key=TEST_SECRET,
        region="us-east-1",
        transport=httpx.MockTransport(handler),
    )
    try:
        client.create_bucket("dcl-redirect")
        raise AssertionError("expected redirect to fail closed")
    except RenterdError as exc:
        assert "HTTP 302" in str(exc)
        assert TEST_SECRET not in str(exc)
        assert TEST_ACCESS not in str(exc)


def test_session_book_provisions_a_per_user_bucket(
    client: TestClient, seeded_host, fake_renterd: FakeRenterd
) -> None:
    token = signup(client)["accessToken"]
    me = client.get("/v1/auth/me", headers=auth_header(token))
    user_id = UUID(me.json()["id"])
    response = client.post("/v1/sessions", headers=auth_header(token))
    assert response.status_code == 201
    storage = response.json()["storage"]
    expected_bucket = bucket_name_for_user(user_id)
    assert storage["provider"] == "renterd"
    assert storage["bucket"] == expected_bucket
    assert storage["prefix"] == "files/"
    assert storage["pathStyle"] is True
    assert storage["mountInsideGuest"] is True
    assert storage["endpoint"] == "http://renterd.test.invalid"
    assert "accessKeyId" not in storage
    assert "secretAccessKey" not in storage
    assert PROVISION_MARKER_KEY in storage["objects"]
    assert "rclone mount" in storage["rclone"]
    assert "--s3-force-path-style" in storage["rclone"]
    assert "--s3-env-auth" in storage["rclone"]
    assert "--s3-region" in storage["rclone"]
    assert storage["envAuth"] is True
    assert storage["rcloneArgs"][0] == "mount"
    assert "--s3-env-auth" in storage["rcloneArgs"]
    assert TEST_SECRET not in storage["rclone"]
    assert TEST_SECRET not in "".join(storage["rcloneArgs"])
    assert fake_renterd.created == [expected_bucket]
    assert fake_renterd.objects == [(expected_bucket, PROVISION_MARKER_KEY)]
    assert "intent" not in response.json()


def test_existing_session_reuses_bucket_without_a_second_create(
    client: TestClient, seeded_host, fake_renterd: FakeRenterd
) -> None:
    token = signup(client)["accessToken"]
    first = client.post("/v1/sessions", headers=auth_header(token))
    second = client.post("/v1/sessions", headers=auth_header(token))
    assert first.status_code == 201
    assert second.status_code == 200
    assert first.json()["storage"]["bucket"] == second.json()["storage"]["bucket"]
    assert len(fake_renterd.created) == 1
    assert seeded_host.id


def test_get_storage_requires_auth_and_returns_guest_grant(
    client: TestClient, fake_renterd: FakeRenterd
) -> None:
    missing = client.get("/v1/storage")
    assert missing.status_code == 401

    token = signup(client)["accessToken"]
    me = client.get("/v1/auth/me", headers=auth_header(token))
    user_id = UUID(me.json()["id"])
    unseen = client.get("/v1/storage", headers=auth_header(token))
    assert unseen.status_code == 404

    created = client.post("/v1/storage", headers=auth_header(token))
    assert created.status_code == 201
    reused = client.post("/v1/storage", headers=auth_header(token))
    assert reused.status_code == 200
    response = client.get("/v1/storage", headers=auth_header(token))
    assert response.status_code == 200
    body = response.json()
    assert body["bucket"] == bucket_name_for_user(user_id)
    assert body["mountInsideGuest"] is True
    assert PROVISION_MARKER_KEY in body["objects"]
    assert "accessKeyId" not in body
    assert "secretAccessKey" not in body
    assert fake_renterd.created == [body["bucket"]]


def test_two_users_get_distinct_buckets(client: TestClient, fake_renterd: FakeRenterd) -> None:
    first = signup(client, "one@example.com")["accessToken"]
    second = signup(client, "two@example.com")["accessToken"]
    one = client.post("/v1/storage", headers=auth_header(first))
    two = client.post("/v1/storage", headers=auth_header(second))
    assert one.status_code == 201
    assert two.status_code == 201
    assert one.json()["bucket"] != two.json()["bucket"]
    assert one.json()["prefix"] == two.json()["prefix"] == "files/"
    assert len(fake_renterd.created) == 2


def test_storage_provision_maps_renterd_failure_to_502(
    client: TestClient, seeded_host, fake_renterd: FakeRenterd
) -> None:
    fake_renterd.error = RenterdError("could not reach renterd S3")
    token = signup(client)["accessToken"]
    response = client.post("/v1/sessions", headers=auth_header(token))
    assert response.status_code == 502
    assert response.json()["detail"] == "renterd storage provision failed"
    assert TEST_SECRET not in response.text
    assert TEST_ACCESS not in response.text
    fake_renterd.error = None
    retry = client.post("/v1/sessions", headers=auth_header(token))
    assert retry.status_code == 201
    assert seeded_host.id


def test_guest_rclone_command_does_not_embed_secrets() -> None:
    args = guest_rclone_args(
        bucket="dcl-example",
        prefix="files/",
        endpoint="http://renterd.test.invalid",
        region="us-east-1",
    )
    command = guest_rclone_mount_command(
        bucket="dcl-example",
        prefix="files/",
        endpoint="http://renterd.test.invalid",
        region="us-east-1",
    )
    assert args[0] == "mount"
    assert args[1] == ":s3:dcl-example/files"
    assert args[2] == "/home/ubuntu/Persistent"
    assert "--s3-env-auth" in args
    assert "--s3-force-path-style" in args
    assert "--s3-region" in args
    assert "us-east-1" in args
    assert "--s3-chunk-size" in args
    assert "120MiB" in args
    assert "--fast-list" in args
    assert command.startswith("rclone mount :s3:dcl-example/files /home/ubuntu/Persistent")
    assert "--vfs-cache-mode full" in command
    assert TEST_SECRET not in command
    assert TEST_ACCESS not in command
    assert TEST_SECRET not in "".join(args)
    assert "AWS_SECRET_ACCESS_KEY" not in command


def test_renterd_modules_do_not_name_backends_or_the_concrete_selector() -> None:
    root = Path(__file__).resolve().parents[1] / "src/matchmaker"
    for relative in ("renterd.py", "storage.py", "routers/storage.py", "routers/sessions.py"):
        text = (root / relative).read_text(encoding="utf-8")
        assert "SingleApprovedHostSelector" not in text
        assert "RankedHostSelector" not in text
        assert "QEMU" not in text
        assert "WHPX" not in text
        assert "VFIO" not in text
        assert "Storj" not in text
        assert TEST_SECRET not in text


def test_renterd_compose_requires_seed_and_exposes_s3() -> None:
    root = Path(__file__).resolve().parents[2] / "renterd"
    compose = (root / "docker-compose.yml").read_text(encoding="utf-8")
    env_example = (root / ".env.example").read_text(encoding="utf-8")
    assert "ghcr.io/siafoundation/renterd:2.9.3" in compose
    assert "RENTERD_SEED:?RENTERD_SEED is required" in compose
    assert "RENTERD_API_PASSWORD:?RENTERD_API_PASSWORD is required" in compose
    assert "RENTERD_S3_ENABLED" in compose
    assert "0.0.0.0:8080" in compose
    assert "--http" in compose
    assert "127.0.0.1:${RENTERD_S3_PORT:-8081}:8080" in compose
    assert "renterd_data:/data" in compose
    assert "RENTERD_NETWORK" in compose
    assert "zen" in env_example
    assert "RENTERD_SEED=" in env_example
    assert "never commit" in env_example.lower() or "Never commit" in env_example
