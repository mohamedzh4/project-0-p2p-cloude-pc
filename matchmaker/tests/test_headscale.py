from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from uuid import UUID

import httpx
from fastapi.testclient import TestClient

from matchmaker.deps import get_headscale_client
from matchmaker.headscale import (
    HOST_ACL_TAG,
    HOST_ACL_TAGS,
    HeadscaleError,
    HttpHeadscaleClient,
    PreauthGrant,
)
from matchmaker.main import app
from tests.helpers import host_auth_header, inventory_payload, signup, auth_header

TEST_API_KEY = "test-headscale-api-key"
TEST_PREAUTH = "tskey-auth-test-not-a-secret"


@dataclass
class FakeHeadscale:
    grant: PreauthGrant
    calls: list[str] = field(default_factory=list)
    error: HeadscaleError | None = None

    def mint_host_preauth(self) -> PreauthGrant:
        self.calls.append("mint")
        if self.error is not None:
            raise self.error
        return self.grant


def _grant() -> PreauthGrant:
    return PreauthGrant(
        login_server="http://headscale.test.invalid",
        key=TEST_PREAUTH,
        user="1",
        reusable=False,
        ephemeral=False,
        expires_at="2026-09-05T12:00:00Z",
        acl_tags=HOST_ACL_TAGS,
    )


def test_http_client_mints_preauth_against_mocked_headscale() -> None:
    recorded: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        recorded.append(request)
        assert request.method == "POST"
        assert request.url.path == "/api/v1/preauthkey"
        assert request.headers["Authorization"] == f"Bearer {TEST_API_KEY}"
        payload = request.content
        body = json.loads(payload)
        assert body["user"] == "1"
        assert body["reusable"] is False
        assert body["ephemeral"] is False
        assert body["aclTags"] == [HOST_ACL_TAG]
        assert body["acl_tags"] == [HOST_ACL_TAG]
        assert "expiration" in body
        return httpx.Response(
            200,
            json={"preAuthKey": {"key": TEST_PREAUTH, "reusable": False, "ephemeral": False}},
        )

    client = HttpHeadscaleClient(
        base_url="http://headscale.test.invalid",
        api_key=TEST_API_KEY,
        user="1",
        login_server="http://login.headscale.test.invalid",
        transport=httpx.MockTransport(handler),
    )
    grant = client.mint_host_preauth()

    assert grant.key == TEST_PREAUTH
    assert grant.login_server == "http://login.headscale.test.invalid"
    assert grant.reusable is False
    assert grant.ephemeral is False
    assert grant.acl_tags == HOST_ACL_TAGS
    assert len(recorded) == 1
    assert recorded[0].url.scheme == "http"


def test_http_client_does_not_follow_redirects() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(302, headers={"Location": "https://evil.example/steal"})

    client = HttpHeadscaleClient(
        base_url="http://headscale.test.invalid",
        api_key=TEST_API_KEY,
        user="1",
        login_server="http://headscale.test.invalid",
        transport=httpx.MockTransport(handler),
    )
    try:
        client.mint_host_preauth()
        raise AssertionError("expected redirect to fail closed")
    except HeadscaleError as exc:
        assert "HTTP 302" in str(exc)
        assert TEST_API_KEY not in str(exc)
        assert TEST_PREAUTH not in str(exc)


def test_http_client_missing_key_in_body_fails_without_leaking_api_key() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, json={"preAuthKey": {"id": "9"}})

    client = HttpHeadscaleClient(
        base_url="http://headscale.test.invalid",
        api_key=TEST_API_KEY,
        user="1",
        login_server="http://headscale.test.invalid",
        transport=httpx.MockTransport(handler),
    )
    try:
        client.mint_host_preauth()
        raise AssertionError("expected missing key to fail")
    except HeadscaleError as exc:
        assert "did not include a key" in str(exc)
        assert TEST_API_KEY not in str(exc)


def test_mint_endpoint_returns_preauth_from_headscale(
    client: TestClient, pending_host
) -> None:
    fake = FakeHeadscale(grant=_grant())
    app.dependency_overrides[get_headscale_client] = lambda: fake
    try:
        response = client.post(
            f"/v1/hosts/{pending_host.id}/mesh/preauth",
            headers=host_auth_header(),
        )
    finally:
        app.dependency_overrides.pop(get_headscale_client, None)

    assert response.status_code == 200
    body = response.json()
    assert body["loginServer"] == "http://headscale.test.invalid"
    assert body["preauthKey"] == TEST_PREAUTH
    assert body["user"] == "1"
    assert body["reusable"] is False
    assert body["ephemeral"] is False
    assert fake.calls == ["mint"]


def test_mint_endpoint_requires_bootstrap_and_known_host(
    client: TestClient, pending_host
) -> None:
    fake = FakeHeadscale(grant=_grant())
    app.dependency_overrides[get_headscale_client] = lambda: fake
    try:
        missing = client.post(f"/v1/hosts/{pending_host.id}/mesh/preauth")
        assert missing.status_code == 401

        user_token = signup(client)["accessToken"]
        as_user = client.post(
            f"/v1/hosts/{pending_host.id}/mesh/preauth",
            headers=auth_header(user_token),
        )
        assert as_user.status_code == 401

        created = client.post("/v1/hosts", headers=host_auth_header(), json=inventory_payload())
        unknown = UUID("00000000-0000-0000-0000-000000000099")
        missing_host = client.post(
            f"/v1/hosts/{unknown}/mesh/preauth",
            headers=host_auth_header(),
        )
        assert missing_host.status_code == 404
        assert created.status_code == 201
        assert fake.calls == []
    finally:
        app.dependency_overrides.pop(get_headscale_client, None)


def test_mint_endpoint_maps_headscale_failure_to_502(
    client: TestClient, pending_host
) -> None:
    fake = FakeHeadscale(grant=_grant(), error=HeadscaleError("could not reach Headscale"))
    app.dependency_overrides[get_headscale_client] = lambda: fake
    try:
        response = client.post(
            f"/v1/hosts/{pending_host.id}/mesh/preauth",
            headers=host_auth_header(),
        )
    finally:
        app.dependency_overrides.pop(get_headscale_client, None)

    assert response.status_code == 502
    assert response.json()["detail"] == "Headscale preauth mint failed"
    assert TEST_PREAUTH not in response.text
    assert TEST_API_KEY not in response.text


def test_headscale_modules_do_not_name_backends_or_the_concrete_selector() -> None:
    root = Path(__file__).resolve().parents[1] / "src/matchmaker"
    for relative in ("headscale.py", "routers/hosts.py"):
        text = (root / relative).read_text(encoding="utf-8")
        assert "SingleApprovedHostSelector" not in text
        assert "RankedHostSelector" not in text
        assert "Firecracker" not in text
        assert "QEMU" not in text
        assert "WHPX" not in text
        assert "VFIO" not in text
        assert "tskey-auth-" not in text
        assert "HEADSCALE_API_KEY=" not in text


def test_mint_endpoint_does_not_advertise_acl_tags_on_the_wire(
    client: TestClient, pending_host
) -> None:
    fake = FakeHeadscale(grant=_grant())
    app.dependency_overrides[get_headscale_client] = lambda: fake
    try:
        response = client.post(
            f"/v1/hosts/{pending_host.id}/mesh/preauth",
            headers=host_auth_header(),
        )
    finally:
        app.dependency_overrides.pop(get_headscale_client, None)

    body = response.json()
    assert response.status_code == 200
    assert "aclTags" not in body
    assert "acl_tags" not in body
    assert set(body) == {
        "loginServer",
        "preauthKey",
        "user",
        "reusable",
        "ephemeral",
        "expiresAt",
    }


_HEADSCALE_DIR = Path(__file__).resolve().parents[2] / "headscale"


def _load_acl_policy() -> dict:
    text = (_HEADSCALE_DIR / "acl.hujson").read_text(encoding="utf-8")
    assert text.strip(), "acl.hujson must be non-empty"
    stripped_lines: list[str] = []
    for line in text.splitlines():
        if "//" in line:
            line = line[: line.index("//")]
        stripped_lines.append(line)
    return json.loads("\n".join(stripped_lines))


def _acl_pairs(policy: dict) -> set[tuple[str, str]]:
    pairs: set[tuple[str, str]] = set()
    for rule in policy["acls"]:
        for src in rule["src"]:
            for dst in rule["dst"]:
                dest = dst.rsplit(":", 1)[0]
                pairs.add((src, dest))
    return pairs


def test_acl_policy_is_default_deny_with_architecture_grants() -> None:
    policy = _load_acl_policy()
    assert "grants" not in policy
    acls = policy["acls"]
    assert isinstance(acls, list) and acls
    for rule in acls:
        assert rule["action"] == "accept"
        joined_src = " ".join(rule["src"])
        joined_dst = " ".join(rule["dst"])
        assert joined_src != "*"
        assert "*:*" not in joined_dst

    pairs = _acl_pairs(policy)
    assert ("tag:host", "tag:matchmaker") in pairs
    assert ("tag:matchmaker", "tag:host") in pairs
    assert ("tag:guest", "tag:matchmaker") in pairs
    assert ("tag:matchmaker", "tag:guest") in pairs
    assert ("tag:user", "tag:guest") in pairs
    assert ("tag:guest", "tag:user") in pairs
    assert ("tag:guest", "tag:exit") in pairs
    assert ("tag:guest", "autogroup:internet") in pairs
    assert ("tag:guest", "tag:host") not in pairs
    assert ("tag:host", "tag:guest") not in pairs
    assert ("tag:host", "autogroup:internet") not in pairs
    assert ("*", "*") not in pairs

    owners = policy["tagOwners"]
    for tag in ("tag:host", "tag:guest", "tag:user", "tag:matchmaker", "tag:exit"):
        assert owners[tag] == ["group:operators"]
    assert policy["groups"]["group:operators"] == ["dcl@"]
    assert policy["autoApprovers"]["exitNode"] == ["tag:exit"]


def test_headscale_config_loads_checked_in_acl_and_exit_node_is_opt_in() -> None:
    config = (_HEADSCALE_DIR / "config.yaml").read_text(encoding="utf-8")
    compose = (_HEADSCALE_DIR / "docker-compose.yml").read_text(encoding="utf-8")
    overlay = (_HEADSCALE_DIR / "docker-compose.exit-node.yml").read_text(encoding="utf-8")

    assert "path: /etc/headscale/acl.hujson" in config
    assert 'path: ""' not in config
    assert "./acl.hujson:/etc/headscale/acl.hujson:ro" in compose
    assert "advertise-exit-node" not in compose
    assert "--advertise-exit-node" in overlay
    assert "HEADSCALE_EXIT_AUTHKEY" in overlay
    assert "network_mode: host" in overlay
