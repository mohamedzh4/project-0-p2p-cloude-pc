from __future__ import annotations

import json
from pathlib import Path
from uuid import UUID, uuid4

import httpx
from fastapi.testclient import TestClient

from matchmaker.deps import get_headscale_client
from matchmaker.headscale import (
    DEFAULT_EXIT_NODE,
    GUEST_ACL_TAG,
    GUEST_ACL_TAGS,
    HOST_ACL_TAG,
    HOST_ACL_TAGS,
    USER_ACL_TAG,
    USER_ACL_TAGS,
    HeadscaleError,
    HttpHeadscaleClient,
    PreauthGrant,
)
from matchmaker.main import app
from matchmaker.mesh import (
    guest_hostname,
    guest_tailscale_up_args,
    mint_session_mesh,
    user_hostname,
    user_tailscale_up_args,
)
from tests.helpers import FakeHeadscale, auth_header, host_auth_header, inventory_payload, signup

TEST_API_KEY = "test-headscale-api-key"
TEST_PREAUTH = "tskey-auth-test-not-a-secret"


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


def test_headscale_config_loads_checked_in_acl_and_exit_overlay_is_guest_path() -> None:
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
    assert "TS_HOSTNAME: dcl-platform-exit" in overlay
    assert DEFAULT_EXIT_NODE == "dcl-platform-exit"


def test_http_client_mints_ephemeral_guest_and_user_tags() -> None:
    recorded: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        recorded.append(request)
        body = json.loads(request.content)
        tag = body["aclTags"][0]
        return httpx.Response(
            200,
            json={"preAuthKey": {"key": f"tskey-auth-test-{tag}", "ephemeral": True}},
        )

    client = HttpHeadscaleClient(
        base_url="http://headscale.test.invalid",
        api_key=TEST_API_KEY,
        user="1",
        login_server="http://login.headscale.test.invalid",
        transport=httpx.MockTransport(handler),
    )
    guest = client.mint_tagged_preauth(acl_tags=GUEST_ACL_TAGS, ephemeral=True)
    user = client.mint_tagged_preauth(acl_tags=USER_ACL_TAGS, ephemeral=True)

    assert guest.ephemeral is True
    assert user.ephemeral is True
    assert guest.acl_tags == GUEST_ACL_TAGS
    assert user.acl_tags == USER_ACL_TAGS
    assert json.loads(recorded[0].content)["ephemeral"] is True
    assert json.loads(recorded[0].content)["aclTags"] == [GUEST_ACL_TAG]
    assert json.loads(recorded[1].content)["aclTags"] == [USER_ACL_TAG]
    assert TEST_API_KEY not in guest.key


def test_http_client_expires_nodes_matching_session_hostnames() -> None:
    recorded: list[httpx.Request] = []
    session_id = uuid4()

    def handler(request: httpx.Request) -> httpx.Response:
        recorded.append(request)
        if request.method == "GET" and request.url.path == "/api/v1/node":
            return httpx.Response(
                200,
                json={
                    "nodes": [
                        {"id": "11", "givenName": guest_hostname(session_id)},
                        {"id": "12", "name": user_hostname(session_id)},
                        {"id": "99", "givenName": "other-node"},
                    ]
                },
            )
        if request.method == "POST" and request.url.path.endswith("/expire"):
            return httpx.Response(200, json={})
        return httpx.Response(500)

    client = HttpHeadscaleClient(
        base_url="http://headscale.test.invalid",
        api_key=TEST_API_KEY,
        user="1",
        login_server="http://login.headscale.test.invalid",
        transport=httpx.MockTransport(handler),
    )
    expired = client.expire_nodes_named(
        (guest_hostname(session_id), user_hostname(session_id))
    )
    assert expired == 2
    paths = [request.url.path for request in recorded]
    assert "/api/v1/node" in paths
    assert "/api/v1/node/11/expire" in paths
    assert "/api/v1/node/12/expire" in paths
    assert "/api/v1/node/99/expire" not in paths


def test_guest_join_args_use_exit_node_user_and_host_do_not() -> None:
    guest = guest_tailscale_up_args(
        login_server="http://headscale.test.invalid",
        auth_key="tskey-auth-test-guest",
        hostname="dcl-guest-example",
        exit_node=DEFAULT_EXIT_NODE,
    )
    user = user_tailscale_up_args(
        login_server="http://headscale.test.invalid",
        auth_key="tskey-auth-test-user",
        hostname="dcl-user-example",
    )
    assert f"--exit-node={DEFAULT_EXIT_NODE}" in guest
    assert "--exit-node-allow-lan-access=false" in guest
    assert not any(item == "--exit-node" or item.startswith("--exit-node=") for item in user)
    assert "--advertise-exit-node" not in " ".join(guest)
    assert "--advertise-tags" not in " ".join(guest)
    assert "--advertise-exit-node" not in " ".join(user)


def test_mint_session_mesh_tags_guest_and_user_ephemeral() -> None:
    fake = FakeHeadscale()
    session_id = uuid4()
    grant = mint_session_mesh(
        fake,
        session_id=session_id,
        login_server="http://headscale.test.invalid",
        exit_node=DEFAULT_EXIT_NODE,
    )
    assert grant.exit_node_applied is True
    assert grant.guest.acl_tags == GUEST_ACL_TAGS
    assert grant.user.acl_tags == USER_ACL_TAGS
    assert grant.guest.ephemeral is True
    assert grant.user.ephemeral is True
    assert grant.guest.hostname == guest_hostname(session_id)
    assert grant.user.hostname == user_hostname(session_id)
    assert grant.guest.preauth_key != grant.user.preauth_key
    assert f"--exit-node={DEFAULT_EXIT_NODE}" in grant.guest.tailscale_up
    assert not any(
        item == "--exit-node" or item.startswith("--exit-node=")
        for item in grant.user.tailscale_up
    )
    assert fake.tagged == [(GUEST_ACL_TAGS, True), (USER_ACL_TAGS, True)]


def test_session_book_mints_guest_and_user_mesh_with_exit_node(
    client: TestClient, seeded_host, fake_headscale: FakeHeadscale
) -> None:
    token = signup(client)["accessToken"]
    response = client.post("/v1/sessions", headers=auth_header(token))
    assert response.status_code == 201
    mesh = response.json()["mesh"]
    session_id = UUID(response.json()["sessionId"])
    assert mesh["loginServer"] == "http://headscale.test.invalid"
    assert mesh["exitNode"] == DEFAULT_EXIT_NODE
    assert mesh["exitNodeApplied"] is True
    assert mesh["guest"]["aclTags"] == [GUEST_ACL_TAG]
    assert mesh["user"]["aclTags"] == [USER_ACL_TAG]
    assert mesh["guest"]["ephemeral"] is True
    assert mesh["user"]["ephemeral"] is True
    assert mesh["guest"]["hostname"] == guest_hostname(session_id)
    assert mesh["user"]["hostname"] == user_hostname(session_id)
    assert mesh["guest"]["preauthKey"].startswith("tskey-auth-test-")
    assert mesh["guest"]["preauthKey"] != mesh["user"]["preauthKey"]
    assert f"--exit-node={DEFAULT_EXIT_NODE}" in mesh["guest"]["tailscaleUp"]
    assert "--advertise-exit-node" not in " ".join(mesh["guest"]["tailscaleUp"])
    assert not any(
        item == "--exit-node" or item.startswith("--exit-node=")
        for item in mesh["user"]["tailscaleUp"]
    )
    assert (GUEST_ACL_TAGS, True) in fake_headscale.tagged
    assert (USER_ACL_TAGS, True) in fake_headscale.tagged
    assert seeded_host.id


def test_session_mesh_mint_failure_is_502_without_occupy(
    client: TestClient, seeded_host, fake_headscale: FakeHeadscale, db
) -> None:
    fake_headscale.error = HeadscaleError("could not reach Headscale")
    token = signup(client)["accessToken"]
    response = client.post("/v1/sessions", headers=auth_header(token))
    assert response.status_code == 502
    assert response.json()["detail"] == "Headscale session mesh mint failed"
    assert TEST_API_KEY not in response.text
    from matchmaker.models import LabSession
    from sqlalchemy import select

    assert db.scalars(select(LabSession)).first() is None
    fake_headscale.error = None
    retry = client.post("/v1/sessions", headers=auth_header(token))
    assert retry.status_code == 201
    assert seeded_host.id


def test_end_session_expires_guest_and_user_hostnames(
    client: TestClient, seeded_host, fake_headscale: FakeHeadscale
) -> None:
    token = signup(client)["accessToken"]
    created = client.post("/v1/sessions", headers=auth_header(token))
    session_id = UUID(created.json()["sessionId"])
    ended = client.post(
        f"/v1/sessions/{session_id}/end", headers=auth_header(token)
    )
    assert ended.status_code == 200
    assert guest_hostname(session_id) in fake_headscale.expired
    assert user_hostname(session_id) in fake_headscale.expired
    assert seeded_host.id


def test_headscale_modules_include_mesh_without_hypervisor_names() -> None:
    root = Path(__file__).resolve().parents[1] / "src/matchmaker"
    for relative in ("headscale.py", "mesh.py", "routers/hosts.py", "routers/sessions.py"):
        text = (root / relative).read_text(encoding="utf-8")
        assert "SingleApprovedHostSelector" not in text
        assert "RankedHostSelector" not in text
        assert "Firecracker" not in text
        assert "QEMU" not in text
        assert "WHPX" not in text
        assert "VFIO" not in text
        assert "tskey-auth-" not in text
        assert "HEADSCALE_API_KEY=" not in text
