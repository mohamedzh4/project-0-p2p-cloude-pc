from __future__ import annotations

from collections.abc import Generator

from sqlalchemy import create_engine
from sqlalchemy.orm import Session, sessionmaker

from matchmaker.config import get_settings

_settings = None
_engine = None
_session_factory = None


def get_engine():
    global _engine, _settings
    if _engine is None:
        _settings = get_settings()
        _engine = create_engine(_settings.database_url, pool_pre_ping=True)
    return _engine


def get_session_factory() -> sessionmaker[Session]:
    global _session_factory
    if _session_factory is None:
        _session_factory = sessionmaker(
            bind=get_engine(),
            autoflush=False,
            autocommit=False,
            expire_on_commit=False,
        )
    return _session_factory


def get_db() -> Generator[Session, None, None]:
    db = get_session_factory()()
    try:
        yield db
    except Exception:
        db.rollback()
        raise
    finally:
        db.close()
