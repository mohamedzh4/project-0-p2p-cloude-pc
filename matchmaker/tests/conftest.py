from __future__ import annotations

import os
import re
from collections.abc import Generator
from pathlib import Path

import pytest
from alembic import command
from alembic.config import Config
from dotenv import load_dotenv
from fastapi.testclient import TestClient
from sqlalchemy import create_engine, text
from sqlalchemy.engine.url import make_url
from sqlalchemy.orm import Session

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

from matchmaker.db import get_engine, get_session_factory  # noqa: E402
from matchmaker.main import app  # noqa: E402
from matchmaker.models import Host, HostStatus, LabTier  # noqa: E402
from tests.helpers import KASM_TOKEN, KASM_URL  # noqa: E402


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
        conn.execute(text("TRUNCATE TABLE sessions, hosts, users RESTART IDENTITY CASCADE"))
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
    )
    db.add(host)
    db.commit()
    db.refresh(host)
    return host


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
