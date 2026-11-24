"""Setup kits downloaded from the website, and the codes inside them."""

from __future__ import annotations

import io
import zipfile
from datetime import datetime, timedelta, timezone

import pytest
from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.config import get_settings
from matchmaker.host_credentials import HOST_SECRET_HEADER
from matchmaker.host_kits import MAX_UNUSED_CODES, mint_code
from matchmaker.models import Host, HostEnrollmentCode, User
from tests.helpers import auth_header, inventory_payload, signup

SECRET_A = "a" * 43
SECRET_B = "b" * 43
PUBLIC_KEY = "ab" * 32


@pytest.fixture
def kits(monkeypatch):
    monkeypatch.setenv("MATCHMAKER_PUBLIC_ORIGIN", "https://lab.test.example/api")
    monkeypatch.setenv("IMAGE_SIGNING_PUBLIC_KEY", PUBLIC_KEY)
    monkeypatch.setenv("HOST_KIT_QCOW2_URL", "https://downloads.test.example/rootfs.qcow2")
    monkeypatch.setenv(
        "HOST_KIT_WINDOWS_INSTALLER_URL", "https://downloads.test.example/host-agent-setup.exe"
    )
    monkeypatch.setenv("HOST_KIT_DEB_URL", "https://downloads.test.example/host-agent.deb")
    get_settings.cache_clear()
    yield
    get_settings.cache_clear()


def _download(client: TestClient, token: str, os_name: str = "windows"):
    return client.post("/v1/host-kits", json={"os": os_name}, headers=auth_header(token))


def _kit_files(response) -> dict[str, str]:
    archive = zipfile.ZipFile(io.BytesIO(response.content))
    return {
        name.split("/", 1)[1]: archive.read(name).decode("utf-8")
        for name in archive.namelist()
    }


def _env(files: dict[str, str]) -> dict[str, str]:
    pairs = (line.split("=", 1) for line in files["friend.env"].splitlines() if "=" in line)
    return {key: value for key, value in pairs}


def _register(client: TestClient, code: str, secret: str = SECRET_A, **overrides):
    return client.post(
        "/v1/hosts",
        json=inventory_payload(**overrides),
        headers={"Authorization": f"Bearer {code}", HOST_SECRET_HEADER: secret},
    )


def test_kits_need_a_login_and_the_server_links(client: TestClient) -> None:
    assert client.post("/v1/host-kits", json={"os": "windows"}).status_code == 401
    token = signup(client)["accessToken"]
    offers = client.get("/v1/host-kits", headers=auth_header(token)).json()["kits"]
    assert offers == [
        {"os": "windows", "available": False},
        {"os": "linux", "available": False},
    ]
    assert _download(client, token).status_code == 503


def test_a_windows_kit_holds_a_code_not_the_shared_token(client: TestClient, kits) -> None:
    token = signup(client)["accessToken"]
    response = _download(client, token)
    assert response.status_code == 200
    assert response.headers["content-type"] == "application/zip"
    assert "dcl-host-setup-windows.zip" in response.headers["content-disposition"]
    files = _kit_files(response)
    assert set(files) == {"setup.cmd", "windows-host-setup.ps1", "friend.env", "README.txt"}
    env = _env(files)
    assert env["HOST_BOOTSTRAP_TOKEN"].startswith("dclk_")
    assert get_settings().host_bootstrap_token not in response.content.decode("latin-1")
    assert env["MATCHMAKER_URL"] == "https://lab.test.example/api"
    assert env["IMAGE_SIGNING_PUBLIC_KEY"] == PUBLIC_KEY
    assert env["IMAGE_STANDARD_DIGEST"] == get_settings().image_standard_digest
    assert env["IMAGE_QCOW2_URL"].endswith("rootfs.qcow2")
    assert env["HOST_AGENT_INSTALLER_URL"].endswith("host-agent-setup.exe")


def test_a_linux_kit_has_the_linux_scripts_and_package_link(client: TestClient, kits) -> None:
    token = signup(client)["accessToken"]
    files = _kit_files(_download(client, token, "linux"))
    assert {"linux-host-setup.sh", "linux-host-uninstall.sh", "friend.env"} <= set(files)
    assert _env(files)["HOST_AGENT_DEB_URL"].endswith("host-agent.deb")
    assert "HOST_AGENT_INSTALLER_URL" not in files["friend.env"]


def test_a_code_registers_one_pc_owned_by_the_user_then_only_that_pc(
    client: TestClient, db: Session, kits
) -> None:
    token = signup(client, email="owner@example.com")["accessToken"]
    code = _env(_kit_files(_download(client, token)))["HOST_BOOTSTRAP_TOKEN"]

    created = _register(client, code)
    assert created.status_code == 201, created.text
    host_id = created.json()["hostId"]
    assert created.json()["status"] == "pending"  # the operator still approves
    host = db.get(Host, host_id)
    owner = db.scalars(select(User).where(User.email == "owner@example.com")).one()
    assert host.owner_user_id == owner.id

    # The same PC keeps using the code: update, heartbeat, polling.
    headers = {"Authorization": f"Bearer {code}", HOST_SECRET_HEADER: SECRET_A}
    assert client.put(f"/v1/hosts/{host_id}", json=inventory_payload(), headers=headers).status_code == 200
    assert client.post(f"/v1/hosts/{host_id}/heartbeat", headers=headers).status_code == 200

    # A second PC with a copy of the kit cannot register.
    again = _register(client, code, secret=SECRET_B)
    assert again.status_code == 403
    # Nor reach another host with it.
    other = client.post(
        "/v1/hosts",
        json=inventory_payload(),
        headers={
            "Authorization": f"Bearer {get_settings().host_bootstrap_token}",
            HOST_SECRET_HEADER: SECRET_B,
        },
    ).json()["hostId"]
    stolen = client.post(
        f"/v1/hosts/{other}/heartbeat",
        headers={"Authorization": f"Bearer {code}", HOST_SECRET_HEADER: SECRET_B},
    )
    assert stolen.status_code == 403


def test_unknown_and_expired_codes_are_refused(client: TestClient, db: Session, kits) -> None:
    assert _register(client, "dclk_" + "z" * 43).status_code == 401
    signup(client, email="late@example.com")
    user = db.scalars(select(User).where(User.email == "late@example.com")).one()
    old = mint_code(
        db, user_id=user.id, lab_os="windows", now=datetime.now(timezone.utc) - timedelta(days=8)
    )
    assert _register(client, old).status_code == 401


def test_a_user_holds_only_a_few_unused_codes(client: TestClient, db: Session, kits) -> None:
    token = signup(client, email="many@example.com")["accessToken"]
    codes = [
        _env(_kit_files(_download(client, token)))["HOST_BOOTSTRAP_TOKEN"] for _ in range(5)
    ]
    user = db.scalars(select(User).where(User.email == "many@example.com")).one()
    rows = db.scalars(select(HostEnrollmentCode).where(HostEnrollmentCode.user_id == user.id)).all()
    assert len(rows) == MAX_UNUSED_CODES
    assert _register(client, codes[0]).status_code == 401  # dropped
    assert _register(client, codes[-1]).status_code == 201


def test_a_new_kit_on_a_pc_an_old_kit_set_up_claims_that_pc(
    client: TestClient, db: Session, kits
) -> None:
    shared = get_settings().host_bootstrap_token
    old = client.post(
        "/v1/hosts",
        json=inventory_payload(),
        headers={"Authorization": f"Bearer {shared}", HOST_SECRET_HEADER: SECRET_A},
    ).json()
    host_id = old["hostId"]
    assert db.get(Host, host_id).owner_user_id is None

    token = signup(client, email="upgrader@example.com")["accessToken"]
    code = _env(_kit_files(_download(client, token)))["HOST_BOOTSTRAP_TOKEN"]

    # Another machine that only knows the id cannot claim it.
    wrong = client.put(
        f"/v1/hosts/{host_id}",
        json=inventory_payload(),
        headers={"Authorization": f"Bearer {code}", HOST_SECRET_HEADER: SECRET_B},
    )
    assert wrong.status_code == 403

    # The PC itself, keeping its id and secret, can.
    headers = {"Authorization": f"Bearer {code}", HOST_SECRET_HEADER: SECRET_A}
    claimed = client.put(f"/v1/hosts/{host_id}", json=inventory_payload(), headers=headers)
    assert claimed.status_code == 200, claimed.text
    assert claimed.json()["status"] == old["status"]
    db.expire_all()
    owner = db.scalars(select(User).where(User.email == "upgrader@example.com")).one()
    assert db.get(Host, host_id).owner_user_id == owner.id
    assert client.post(f"/v1/hosts/{host_id}/heartbeat", headers=headers).status_code == 200

    # A PC that belongs to someone else cannot be claimed with your code.
    other_token = signup(client, email="someone@example.com")["accessToken"]
    other_code = _env(_kit_files(_download(client, other_token)))["HOST_BOOTSTRAP_TOKEN"]
    taken = client.put(
        f"/v1/hosts/{host_id}",
        json=inventory_payload(),
        headers={"Authorization": f"Bearer {other_code}", HOST_SECRET_HEADER: SECRET_A},
    )
    assert taken.status_code == 403
