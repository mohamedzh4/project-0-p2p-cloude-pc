from __future__ import annotations

from matchmaker.config import Settings


def _required_env(monkeypatch) -> None:
    monkeypatch.setenv("DATABASE_URL", "postgresql+psycopg://matchmaker:x@localhost:5432/matchmaker")
    monkeypatch.setenv("JWT_SECRET", "test-jwt-secret")
    monkeypatch.setenv("HOST_BOOTSTRAP_TOKEN", "test-host-bootstrap-token")
    monkeypatch.setenv("HOST_APPROVE_TOKEN", "test-host-approve-token")
    monkeypatch.delenv("HOST_AGENT_PROVISION_URL", raising=False)


def test_credits_per_minute_is_required(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.delenv("CREDITS_PER_MINUTE", raising=False)
    try:
        Settings.from_env()
        raise AssertionError("expected CREDITS_PER_MINUTE to fail fast")
    except RuntimeError as exc:
        assert "CREDITS_PER_MINUTE is required" in str(exc)


def test_credits_per_minute_must_be_a_non_negative_integer(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "nope")
    try:
        Settings.from_env()
        raise AssertionError("expected CREDITS_PER_MINUTE to fail fast")
    except RuntimeError as exc:
        assert "CREDITS_PER_MINUTE must be an integer" in str(exc)

    monkeypatch.setenv("CREDITS_PER_MINUTE", "-1")
    try:
        Settings.from_env()
        raise AssertionError("expected CREDITS_PER_MINUTE to fail fast")
    except RuntimeError as exc:
        assert "CREDITS_PER_MINUTE must be non-negative" in str(exc)


def test_credits_per_minute_zero_is_allowed(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "0")
    settings = Settings.from_env()
    assert settings.credits_per_minute == 0


def test_stale_sweep_interval_defaults_and_zero_disables(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.delenv("STALE_SESSION_SWEEP_INTERVAL_SECONDS", raising=False)
    settings = Settings.from_env()
    assert settings.stale_session_sweep_interval_seconds == 15

    monkeypatch.setenv("STALE_SESSION_SWEEP_INTERVAL_SECONDS", "0")
    assert Settings.from_env().stale_session_sweep_interval_seconds == 0


def test_stale_sweep_interval_must_be_a_non_negative_integer(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.setenv("STALE_SESSION_SWEEP_INTERVAL_SECONDS", "nope")
    try:
        Settings.from_env()
        raise AssertionError("expected STALE_SESSION_SWEEP_INTERVAL_SECONDS to fail fast")
    except RuntimeError as exc:
        assert "STALE_SESSION_SWEEP_INTERVAL_SECONDS must be an integer" in str(exc)

    monkeypatch.setenv("STALE_SESSION_SWEEP_INTERVAL_SECONDS", "-1")
    try:
        Settings.from_env()
        raise AssertionError("expected STALE_SESSION_SWEEP_INTERVAL_SECONDS to fail fast")
    except RuntimeError as exc:
        assert "STALE_SESSION_SWEEP_INTERVAL_SECONDS must be non-negative" in str(exc)
