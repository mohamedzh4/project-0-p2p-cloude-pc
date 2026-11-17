"""Per-user storage keys (issue 1): a guest never gets the master key."""

from __future__ import annotations

import json
from collections.abc import Generator
from dataclasses import dataclass, field

import httpx
import pytest
from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from matchmaker.config import Settings, get_settings
from matchmaker.deps import get_storage_credential_issuer
from matchmaker.main import app
from matchmaker.models import Host, UserStorage
from matchmaker.renterd import _sizes_from_list_xml
from matchmaker.storage import location_for_user, shared_prefix_for_user
from matchmaker.storage_credentials import (
    GuestStorageCredentials,
    R2TempCredentialIssuer,
    StaticCredentialIssuer,
    StorageCredentialError,
)
from tests.helpers import FakeHostProvisioner, FakeRenterd, auth_header, signup

API_TOKEN = "cf-api-token-value-never-leaves"
PARENT_KEY = "parent-access-key-id"


def _issuer(handler) -> R2TempCredentialIssuer:
    return R2TempCredentialIssuer(
        account_id="acct123",
        api_token=API_TOKEN,
        parent_access_key_id=PARENT_KEY,
        transport=httpx.MockTransport(handler),
    )


def _ok(request: httpx.Request) -> httpx.Response:
    return httpx.Response(
        200,
        json={
            "success": True,
            "result": {
                "accessKeyId": "temp-access",
                "secretAccessKey": "temp-secret",
                "sessionToken": "temp-session",
            },
        },
    )


def test_r2_issuer_scopes_the_request_to_one_bucket_and_prefix() -> None:
    seen: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        return _ok(request)

    creds = _issuer(handler).issue(
        bucket="dcl-files", prefix="users/u1/files/", read_only=False, ttl_seconds=3600
    )
    assert creds == GuestStorageCredentials("temp-access", "temp-secret", "temp-session")
    request = seen[0]
    assert request.method == "POST"
    assert str(request.url) == (
        "https://api.cloudflare.com/client/v4/accounts/acct123/r2/temp-access-credentials"
    )
    assert request.headers["Authorization"] == f"Bearer {API_TOKEN}"
    assert json.loads(request.content) == {
        "bucket": "dcl-files",
        "parentAccessKeyId": PARENT_KEY,
        "permission": "object-read-write",
        "ttlSeconds": 3600,
        "prefixes": ["users/u1/files/"],
    }


def test_r2_issuer_read_only_and_ttl_cap() -> None:
    seen: list[dict] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(json.loads(request.content))
        return _ok(request)

    _issuer(handler).issue(bucket="b", prefix="users/u/files/", read_only=True, ttl_seconds=10**9)
    assert seen[0]["permission"] == "object-read-only"
    assert seen[0]["ttlSeconds"] == 604800


def test_r2_issuer_refuses_an_empty_prefix() -> None:
    def handler(request: httpx.Request) -> httpx.Response:  # pragma: no cover
        raise AssertionError("must not call Cloudflare")

    for prefix in ("", "/", "  "):
        with pytest.raises(StorageCredentialError):
            _issuer(handler).issue(bucket="b", prefix=prefix, read_only=False, ttl_seconds=60)


@pytest.mark.parametrize(
    "response",
    [
        httpx.Response(403, json={"success": False}),
        httpx.Response(200, text="not json"),
        httpx.Response(200, json={"result": None}),
        httpx.Response(200, json={"result": {"accessKeyId": "a", "secretAccessKey": "s"}}),
    ],
)
def test_r2_issuer_errors_never_include_the_api_token(response: httpx.Response) -> None:
    with pytest.raises(StorageCredentialError) as raised:
        _issuer(lambda request: response).issue(
            bucket="b", prefix="users/u/files/", read_only=False, ttl_seconds=60
        )
    assert API_TOKEN not in str(raised.value)


def test_r2_issuer_network_failure_is_a_credential_error() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("down")

    with pytest.raises(StorageCredentialError):
        _issuer(handler).issue(bucket="b", prefix="users/u/files/", read_only=False, ttl_seconds=60)


def test_credentials_repr_hides_secrets() -> None:
    text = repr(GuestStorageCredentials("id", "very-secret", "very-token"))
    assert "very-secret" not in text
    assert "very-token" not in text


def test_static_issuer_is_marked_unscoped() -> None:
    assert StaticCredentialIssuer(access_key="a", secret_key="s").scoped is False
    assert R2TempCredentialIssuer.scoped is True


def test_shared_bucket_gives_each_user_their_own_prefix() -> None:
    from uuid import uuid4

    a, b = uuid4(), uuid4()
    assert location_for_user(a, shared_bucket="dcl-files") == ("dcl-files", f"users/{a}/files/")
    assert shared_prefix_for_user(a) != shared_prefix_for_user(b)
    assert not shared_prefix_for_user(a).startswith(shared_prefix_for_user(b))
    bucket, prefix = location_for_user(a, shared_bucket=None)
    assert bucket == f"dcl-{a}"
    assert prefix == "files/"


def test_list_xml_sizes_and_continuation() -> None:
    xml = b"""<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
      <Contents><Key>a</Key><Size>10</Size></Contents>
      <Contents><Key>b</Key><Size>32</Size></Contents>
      <NextContinuationToken>next-page</NextContinuationToken>
    </ListBucketResult>"""
    assert _sizes_from_list_xml(xml) == ([10, 32], "next-page")
    assert _sizes_from_list_xml(b"<ListBucketResult/>") == ([], None)


def _settings_with(monkeypatch, **env: str) -> Settings:
    for key, value in env.items():
        monkeypatch.setenv(key, value)
    return Settings.from_env()


def test_r2_mode_requires_account_and_bucket_token_is_optional(monkeypatch) -> None:
    monkeypatch.setenv("STORAGE_CREDENTIALS", "r2")
    with pytest.raises(RuntimeError, match="R2_ACCOUNT_ID"):
        Settings.from_env()
    monkeypatch.setenv("R2_ACCOUNT_ID", "acct")
    with pytest.raises(RuntimeError, match="S3_BUCKET"):
        Settings.from_env()
    monkeypatch.setenv("S3_BUCKET", "dcl-files")
    assert Settings.from_env().r2_api_token is None  # the token is optional
    monkeypatch.setenv("R2_API_TOKEN", "cf-secret-value")
    settings = _settings_with(monkeypatch, S3_BUCKET="dcl-files", STORAGE_QUOTA_GB="5")
    assert settings.storage_credentials_mode == "r2"
    assert settings.storage_quota_bytes == 5 * 1024**3
    assert "cf-secret-value" not in repr(settings)


def test_unknown_storage_mode_is_rejected(monkeypatch) -> None:
    monkeypatch.setenv("STORAGE_CREDENTIALS", "magic")
    with pytest.raises(RuntimeError, match="STORAGE_CREDENTIALS"):
        Settings.from_env()


@dataclass
class RecordingIssuer:
    calls: list[dict] = field(default_factory=list)
    error: StorageCredentialError | None = None

    def issue(self, *, bucket: str, prefix: str, read_only: bool, ttl_seconds: int):
        self.calls.append(
            {"bucket": bucket, "prefix": prefix, "read_only": read_only, "ttl": ttl_seconds}
        )
        if self.error is not None:
            raise self.error
        return GuestStorageCredentials(
            f"temp-access-{len(self.calls)}", "temp-secret", "temp-session"
        )


@pytest.fixture
def issuer() -> Generator[RecordingIssuer, None, None]:
    fake = RecordingIssuer()
    app.dependency_overrides[get_storage_credential_issuer] = lambda: fake
    yield fake
    app.dependency_overrides.pop(get_storage_credential_issuer, None)


@pytest.fixture
def shared_bucket(monkeypatch) -> Generator[str, None, None]:
    monkeypatch.setenv("S3_BUCKET", "dcl-files")
    get_settings.cache_clear()
    yield "dcl-files"
    monkeypatch.delenv("S3_BUCKET")
    get_settings.cache_clear()


def _book_and_redeem(client: TestClient, provisioner: FakeHostProvisioner, email: str):
    account = signup(client, email=email)
    booked = client.post("/v1/sessions", headers=auth_header(account["accessToken"]))
    assert booked.status_code == 201, booked.text
    hint = provisioner.mesh_calls[-1]
    redeemed = client.post(
        "/v1/guest/bootstrap", headers={"Authorization": f"Bearer {hint.bootstrap_token}"}
    )
    return account, booked, redeemed


def test_guest_gets_a_key_scoped_to_its_own_users_prefix(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    fake_renterd: FakeRenterd,
    issuer: RecordingIssuer,
    shared_bucket: str,
    db: Session,
) -> None:
    account, booked, redeemed = _book_and_redeem(client, fake_provisioner, "ada@example.com")
    assert redeemed.status_code == 200, redeemed.text
    user_id = account["user"]["id"]
    prefix = f"users/{user_id}/files/"

    assert issuer.calls == [
        {"bucket": "dcl-files", "prefix": prefix, "read_only": False, "ttl": 43200}
    ]
    storage = redeemed.json()["storage"]
    assert storage["accessKeyId"] == "temp-access-1"
    assert storage["sessionToken"] == "temp-session"
    assert storage["readOnly"] is False
    assert storage["bucket"] == "dcl-files"
    assert storage["prefix"] == prefix
    assert "--s3-no-check-bucket" in storage["rcloneArgs"]
    assert "--read-only" not in storage["rcloneArgs"]
    # The Matchmaker's own key is nowhere in what the guest receives.
    assert "test-renterd-access" not in redeemed.text
    assert "test-renterd-secret-key" not in redeemed.text
    # No bucket is created per user; only the user's marker is written.
    assert fake_renterd.created == []
    assert fake_renterd.objects == [("dcl-files", f"{prefix}.dcl/provisioned")]
    row = db.get(UserStorage, user_id)
    assert (row.bucket, row.prefix) == ("dcl-files", prefix)
    assert booked.json()["storage"]["prefix"] == prefix


def test_two_users_share_the_bucket_but_not_the_prefix(
    client: TestClient,
    make_approved_host,
    fake_provisioner: FakeHostProvisioner,
    issuer: RecordingIssuer,
    shared_bucket: str,
) -> None:
    make_approved_host(display_name="one")
    make_approved_host(display_name="two")
    _book_and_redeem(client, fake_provisioner, "a@example.com")
    _book_and_redeem(client, fake_provisioner, "b@example.com")
    first, second = issuer.calls
    assert first["bucket"] == second["bucket"] == "dcl-files"
    assert first["prefix"] != second["prefix"]


def test_over_quota_user_gets_a_read_only_key(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    fake_renterd: FakeRenterd,
    issuer: RecordingIssuer,
    shared_bucket: str,
) -> None:
    account = signup(client, email="big@example.com")
    prefix = f"users/{account['user']['id']}/files/"
    fake_renterd.usage[("dcl-files", prefix)] = 20 * 1024**3
    client.post("/v1/sessions", headers=auth_header(account["accessToken"]))
    hint = fake_provisioner.mesh_calls[-1]
    redeemed = client.post(
        "/v1/guest/bootstrap", headers={"Authorization": f"Bearer {hint.bootstrap_token}"}
    )
    assert issuer.calls[0]["read_only"] is True
    assert redeemed.json()["storage"]["readOnly"] is True
    assert "--read-only" in redeemed.json()["storage"]["rcloneArgs"]


def test_issuer_failure_is_a_502_without_details(
    client: TestClient,
    seeded_host: Host,
    fake_provisioner: FakeHostProvisioner,
    issuer: RecordingIssuer,
) -> None:
    issuer.error = StorageCredentialError("Cloudflare said no")
    _, _, redeemed = _book_and_redeem(client, fake_provisioner, "c@example.com")
    assert redeemed.status_code == 502
    assert redeemed.json()["detail"] == "storage credentials could not be issued"


# --- local signing (no Cloudflare API call) --------------------------------


def _local_issuer():
    from matchmaker.storage_credentials import R2LocalCredentialIssuer

    return R2LocalCredentialIssuer(
        account_id="acct123",
        endpoint="https://acct123.r2.cloudflarestorage.com",
        parent_access_key_id=PARENT_KEY,
        parent_secret_access_key="parent-secret-never-leaves",
    )


def _decode_session(creds: GuestStorageCredentials) -> tuple[dict, dict, str]:
    import base64

    raw = base64.b64decode(creds.session_token).decode()
    assert raw.startswith("jwt/")
    token = raw[4:]
    header, claims, _signature = token.split(".")

    def part(value: str) -> dict:
        return json.loads(base64.urlsafe_b64decode(value + "=" * (-len(value) % 4)))

    return part(header), part(claims), token


def test_local_issuer_follows_cloudflares_signing_scheme() -> None:
    import base64
    import hashlib
    import hmac

    creds = _local_issuer().issue(
        bucket="dcl-files", prefix="users/u1/files/", read_only=False, ttl_seconds=3600, now=1000
    )
    header, claims, token = _decode_session(creds)
    assert header == {"alg": "HS256", "typ": "JWT"}
    assert claims == {
        "bucket": "dcl-files",
        "scope": "object-read-write",
        "paths": {"prefixPaths": ["users/u1/files/"], "objectPaths": []},
        "sub": "acct123",
        "iss": PARENT_KEY,
        "aud": "acct123.r2.cloudflarestorage.com",
        "iat": 1000,
        "exp": 4600,
    }
    # Access key id is the parent's; the secret is the SHA-256 of the JWT.
    assert creds.access_key_id == PARENT_KEY
    assert creds.secret_access_key == hashlib.sha256(token.encode()).hexdigest()
    signing_input, _, signature = token.rpartition(".")
    expected = hmac.new(b"parent-secret-never-leaves", signing_input.encode(), hashlib.sha256)
    assert signature == base64.urlsafe_b64encode(expected.digest()).rstrip(b"=").decode()


def test_local_issuer_never_hands_out_the_parent_secret() -> None:
    creds = _local_issuer().issue(
        bucket="b", prefix="users/u/files/", read_only=False, ttl_seconds=60
    )
    import base64

    assert "parent-secret-never-leaves" not in creds.secret_access_key
    assert "parent-secret-never-leaves" not in base64.b64decode(creds.session_token).decode()


def test_local_issuer_read_only_ttl_cap_and_prefix_guard() -> None:
    issuer = _local_issuer()
    _, claims, _ = _decode_session(
        issuer.issue(bucket="b", prefix="users/u/files/", read_only=True, ttl_seconds=10**9, now=0)
    )
    assert claims["scope"] == "object-read-only"
    assert claims["exp"] == 604800
    for prefix in ("", "/", "  "):
        with pytest.raises(StorageCredentialError):
            issuer.issue(bucket="b", prefix=prefix, read_only=False, ttl_seconds=60)


def test_two_users_get_different_keys_for_different_prefixes() -> None:
    issuer = _local_issuer()
    a = issuer.issue(bucket="b", prefix="users/a/files/", read_only=False, ttl_seconds=60, now=5)
    b = issuer.issue(bucket="b", prefix="users/b/files/", read_only=False, ttl_seconds=60, now=5)
    assert a.secret_access_key != b.secret_access_key
    assert _decode_session(a)[1]["paths"]["prefixPaths"] == ["users/a/files/"]
    assert _decode_session(b)[1]["paths"]["prefixPaths"] == ["users/b/files/"]


def test_r2_mode_without_api_token_uses_local_signing(monkeypatch) -> None:
    from matchmaker.storage_credentials import R2LocalCredentialIssuer

    monkeypatch.setenv("STORAGE_CREDENTIALS", "r2")
    monkeypatch.setenv("R2_ACCOUNT_ID", "acct123")
    monkeypatch.setenv("S3_BUCKET", "dcl-files")
    get_settings.cache_clear()
    try:
        assert isinstance(get_storage_credential_issuer(), R2LocalCredentialIssuer)
        monkeypatch.setenv("R2_API_TOKEN", "cf-secret-value")
        get_settings.cache_clear()
        assert isinstance(get_storage_credential_issuer(), R2TempCredentialIssuer)
    finally:
        monkeypatch.delenv("STORAGE_CREDENTIALS")
        monkeypatch.delenv("R2_ACCOUNT_ID")
        monkeypatch.delenv("S3_BUCKET")
        monkeypatch.delenv("R2_API_TOKEN", raising=False)
        get_settings.cache_clear()


def test_session_token_is_a_signed_header() -> None:
    from matchmaker.renterd import _sigv4_headers

    headers = _sigv4_headers(
        method="GET",
        url="https://acct.r2.cloudflarestorage.com/b",
        body=b"",
        access_key="access-key-id-0000",
        secret_key="secret-access-key-0000",
        region="auto",
        session_token="temp-session",
    )
    assert headers["x-amz-security-token"] == "temp-session"
    assert "SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token" in (
        headers["Authorization"]
    )


def test_r2_mounts_use_small_upload_parts() -> None:
    from matchmaker.storage import guest_rclone_args

    r2 = guest_rclone_args(
        bucket="dcl-files",
        prefix="users/u/files/",
        endpoint="https://acct123.r2.cloudflarestorage.com",
        region="auto",
    )
    assert "--s3-chunk-size" not in r2
    assert "--s3-no-check-bucket" in r2
    other = guest_rclone_args(
        bucket="b", prefix="files/", endpoint="http://renterd.test.invalid", region="us-east-1"
    )
    assert "120MiB" in other
