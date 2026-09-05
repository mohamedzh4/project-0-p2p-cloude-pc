from __future__ import annotations

import os
import re
from collections.abc import Callable, Generator
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import pytest
from alembic import command
from alembic.config import Config
from dotenv import load_dotenv
from fastapi.testclient import TestClient
from sqlalchemy import create_engine, text
from sqlalchemy.engine.url import make_url
from sqlalchemy.orm import Session

from tests.image_fixtures import (
    PRO_DIGEST,
    PRO_SIGNATURE,
    STANDARD_DIGEST,
    STANDARD_SIGNATURE,
)

ROOT = Path(__file__).resolve().parent.parent
load_dotenv(ROOT / ".env")

_test_url = os.environ.get("TEST_DATABASE_URL")
if not _test_url:
    raise RuntimeError("TEST_DATABASE_URL is required")
os.environ["DATABASE_URL"] = _test_url

if not os.environ.get("JWT_SECRET"):
    raise RuntimeError("JWT_SECRET is required")

if not os.environ.get("HOST_BOOTSTRAP_TOKEN"):
    os.environ["HOST_BOOTSTRAP_TOKEN"] = "test-host-bootstrap-token"
if not os.environ.get("HOST_APPROVE_TOKEN"):
    os.environ["HOST_APPROVE_TOKEN"] = "test-host-approve-token"
if not os.environ.get("CREDITS_PER_MINUTE"):
    os.environ["CREDITS_PER_MINUTE"] = "1"
# Pytest drives reclaim from the request path; do not race TRUNCATE with a loop.
os.environ["STALE_SESSION_SWEEP_INTERVAL_SECONDS"] = "0"
# Local-dev image vectors (same as host-agent image::tests). Not a production key.
if not os.environ.get("IMAGE_STANDARD_DIGEST"):
    os.environ["IMAGE_STANDARD_DIGEST"] = STANDARD_DIGEST
if not os.environ.get("IMAGE_STANDARD_SIGNATURE"):
    os.environ["IMAGE_STANDARD_SIGNATURE"] = STANDARD_SIGNATURE
if not os.environ.get("IMAGE_PRO_DIGEST"):
    os.environ["IMAGE_PRO_DIGEST"] = PRO_DIGEST
if not os.environ.get("IMAGE_PRO_SIGNATURE"):
    os.environ["IMAGE_PRO_SIGNATURE"] = PRO_SIGNATURE
# A local .env must not point pytest at a live agent; tests inject a fake or bind one.
os.environ.pop("HOST_AGENT_PROVISION_URL", None)
# Never call a live Headscale from unit tests. Mint tests use a fake or MockTransport.
os.environ["HEADSCALE_URL"] = "http://headscale.test.invalid"
os.environ["HEADSCALE_LOGIN_SERVER"] = "http://headscale.test.invalid"
os.environ["HEADSCALE_API_KEY"] = "test-headscale-api-key"
os.environ["HEADSCALE_USER"] = "1"

from matchmaker.db import get_engine, get_session_factory  # noqa: E402
from matchmaker.deps import get_host_provisioner  # noqa: E402
from matchmaker.main import app  # noqa: E402
from matchmaker.models import Host, HostStatus, LabTier  # noqa: E402
from tests.helpers import KASM_TOKEN, KASM_URL, FakeHostProvisioner, PROVISION_URL  # noqa: E402


def _ensure_database(url: str) -> None:
    parsed = make_url(url)
    dbname = parsed.database
    if not dbname or not re.fullmatch(r"[A-Za-z0-9_]+", dbname):
        raise RuntimeError("TEST_DATABASE_URL database name is invalid")
    admin_url = parsed.set(database="postgres")
    engine = create_engine(admin_url, isolation_level="AUTOCOMMIT")
    try:
        with engine.connect() as conn:
            exists = conn.execute(
                text("SELECT 1 FROM pg_database WHERE datname = :name"),
                {"name": dbname},
            ).scalar()
            if not exists:
                conn.execute(text(f'CREATE DATABASE "{dbname}"'))
    finally:
        engine.dispose()


_ensure_database(_test_url)
_alembic = Config(str(ROOT / "alembic.ini"))
command.upgrade(_alembic, "head")


@pytest.fixture(autouse=True)
def clean_tables() -> Generator[None, None, None]:
    engine = get_engine()
    with engine.begin() as conn:
        conn.execute(
            text("TRUNCATE TABLE ledger_entries, sessions, hosts, users RESTART IDENTITY CASCADE")
        )
    yield


@pytest.fixture
def db() -> Generator[Session, None, None]:
    session = get_session_factory()()
    try:
        yield session
    finally:
        session.close()


@pytest.fixture
def client() -> TestClient:
    return TestClient(app)


@pytest.fixture
def fake_provisioner() -> Generator[FakeHostProvisioner, None, None]:
    fake = FakeHostProvisioner(connection_url=PROVISION_URL)
    app.dependency_overrides[get_host_provisioner] = lambda: fake
    try:
        yield fake
    finally:
        app.dependency_overrides.pop(get_host_provisioner, None)


@pytest.fixture
def seeded_host(db: Session) -> Host:
    host = Host(
        display_name="test-host",
        lab_tier=LabTier.STANDARD,
        shared_logical_cores=4,
        shared_memory_bytes=8 * 1024 * 1024 * 1024,
        share_gpu=False,
        shared_gpu_id=None,
        kasm_url=KASM_URL,
        kasm_token=KASM_TOKEN,
        status=HostStatus.APPROVED,
        last_seen_at=datetime.now(timezone.utc),
    )
    db.add(host)
    db.commit()
    db.refresh(host)
    return host


_FRESH_NOW = object()


@pytest.fixture
def make_approved_host(db: Session) -> Callable[..., Host]:
    def _make(
        *,
        display_name: str = "ranked-host",
        lab_tier: str = LabTier.STANDARD,
        shared_logical_cores: int = 4,
        shared_memory_bytes: int = 8 * 1024 * 1024 * 1024,
        share_gpu: bool = False,
        shared_gpu_id: str | None = None,
        status: str = HostStatus.APPROVED,
        last_seen_at: Any = _FRESH_NOW,
        kasm_url: str = KASM_URL,
        kasm_token: str | None = KASM_TOKEN,
    ) -> Host:
        seen = datetime.now(timezone.utc) if last_seen_at is _FRESH_NOW else last_seen_at
        host = Host(
            display_name=display_name,
            lab_tier=lab_tier,
            shared_logical_cores=shared_logical_cores,
            shared_memory_bytes=shared_memory_bytes,
            share_gpu=share_gpu,
            shared_gpu_id=shared_gpu_id,
            kasm_url=kasm_url,
            kasm_token=kasm_token,
            status=status,
            last_seen_at=seen,
        )
        db.add(host)
        db.commit()
        db.refresh(host)
        return host

    return _make


@pytest.fixture
def pending_host(db: Session) -> Host:
    host = Host(
        display_name="pending-host",
        lab_tier=LabTier.STANDARD,
        shared_logical_cores=2,
        shared_memory_bytes=4 * 1024 * 1024 * 1024,
        share_gpu=False,
        shared_gpu_id=None,
        kasm_url=KASM_URL,
        kasm_token=KASM_TOKEN,
        status=HostStatus.PENDING,
    )
    db.add(host)
    db.commit()
    db.refresh(host)
    return host
