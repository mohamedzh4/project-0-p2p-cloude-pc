from __future__ import annotations

from matchmaker.config import Settings
from tests.image_fixtures import (
    PRO_DIGEST,
    PRO_SIGNATURE,
    STANDARD_DIGEST,
    STANDARD_SIGNATURE,
)


def _required_env(monkeypatch) -> None:
    monkeypatch.setenv("DATABASE_URL", "postgresql+psycopg://matchmaker:x@localhost:5432/matchmaker")
    monkeypatch.setenv("JWT_SECRET", "test-jwt-secret")
    monkeypatch.setenv("HOST_BOOTSTRAP_TOKEN", "test-host-bootstrap-token")
    monkeypatch.setenv("HOST_APPROVE_TOKEN", "test-host-approve-token")
    monkeypatch.setenv("IMAGE_STANDARD_DIGEST", STANDARD_DIGEST)
    monkeypatch.setenv("IMAGE_STANDARD_SIGNATURE", STANDARD_SIGNATURE)
    monkeypatch.setenv("IMAGE_PRO_DIGEST", PRO_DIGEST)
    monkeypatch.setenv("IMAGE_PRO_SIGNATURE", PRO_SIGNATURE)
    monkeypatch.setenv("HEADSCALE_URL", "http://headscale.test.invalid")
    monkeypatch.setenv("HEADSCALE_LOGIN_SERVER", "http://headscale.test.invalid")
    monkeypatch.setenv("HEADSCALE_API_KEY", "test-headscale-api-key")
    monkeypatch.setenv("HEADSCALE_USER", "1")
    monkeypatch.setenv("RENTERD_S3_ENDPOINT", "http://renterd.test.invalid")
    monkeypatch.setenv("RENTERD_S3_ACCESS_KEY", "test-renterd-access")
    monkeypatch.setenv("RENTERD_S3_SECRET_KEY", "test-renterd-secret-key")
    monkeypatch.setenv("RENTERD_S3_REGION", "us-east-1")
    monkeypatch.delenv("RENTERD_S3_GUEST_ENDPOINT", raising=False)
    monkeypatch.delenv("S3_ENDPOINT", raising=False)
    monkeypatch.delenv("S3_ACCESS_KEY", raising=False)
    monkeypatch.delenv("S3_SECRET_KEY", raising=False)
    monkeypatch.delenv("S3_REGION", raising=False)
    monkeypatch.delenv("S3_GUEST_ENDPOINT", raising=False)
    monkeypatch.delenv("HOST_AGENT_PROVISION_URL", raising=False)
    monkeypatch.delenv("HEADSCALE_EXIT_NODE", raising=False)


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


def test_image_digest_and_signature_are_required(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.delenv("IMAGE_STANDARD_DIGEST", raising=False)
    try:
        Settings.from_env()
        raise AssertionError("expected IMAGE_STANDARD_DIGEST to fail fast")
    except RuntimeError as exc:
        assert "IMAGE_STANDARD_DIGEST is required" in str(exc)


def test_image_digest_must_be_sha256_hex(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.setenv("IMAGE_STANDARD_DIGEST", "not-a-digest")
    try:
        Settings.from_env()
        raise AssertionError("expected IMAGE_STANDARD_DIGEST to fail fast")
    except RuntimeError as exc:
        assert "64-character SHA-256 hex digest" in str(exc)


def test_image_signature_must_be_64_byte_ed25519(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.setenv("IMAGE_STANDARD_SIGNATURE", "not-base64!!!")
    try:
        Settings.from_env()
        raise AssertionError("expected IMAGE_STANDARD_SIGNATURE to fail fast")
    except RuntimeError as exc:
        assert "64-byte Ed25519 signature" in str(exc)


def test_image_env_accepts_sha256_prefix(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.setenv("IMAGE_STANDARD_DIGEST", f"sha256:{STANDARD_DIGEST}")
    settings = Settings.from_env()
    assert settings.image_standard_digest == STANDARD_DIGEST
    assert settings.image_standard_signature == STANDARD_SIGNATURE
    assert settings.image_pro_digest == PRO_DIGEST
    assert settings.image_pro_signature == PRO_SIGNATURE


def test_headscale_env_is_required(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.delenv("HEADSCALE_API_KEY", raising=False)
    try:
        Settings.from_env()
        raise AssertionError("expected HEADSCALE_API_KEY to fail fast")
    except RuntimeError as exc:
        assert "HEADSCALE_API_KEY is required" in str(exc)


def test_headscale_url_must_be_http(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.setenv("HEADSCALE_URL", "not-a-url")
    try:
        Settings.from_env()
        raise AssertionError("expected HEADSCALE_URL to fail fast")
    except RuntimeError as exc:
        assert "HEADSCALE_URL must be an http or https URL" in str(exc)


def test_headscale_secrets_are_redacted_in_repr(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    settings = Settings.from_env()
    dumped = repr(settings)
    assert "test-headscale-api-key" not in dumped
    assert "test-jwt-secret" not in dumped
    assert "test-host-bootstrap-token" not in dumped
    assert settings.headscale_url == "http://headscale.test.invalid"
    assert settings.headscale_login_server == "http://headscale.test.invalid"
    assert settings.headscale_user == "1"
    assert settings.headscale_exit_node == "dcl-platform-exit"
    assert settings.renterd_s3_endpoint == "http://renterd.test.invalid"
    assert settings.renterd_s3_guest_endpoint == "http://renterd.test.invalid"
    assert settings.renterd_s3_region == "us-east-1"
    assert "test-renterd-secret-key" not in dumped
    assert "test-renterd-access" not in dumped


def test_headscale_exit_node_defaults_to_platform_overlay_hostname(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.delenv("HEADSCALE_EXIT_NODE", raising=False)
    assert Settings.from_env().headscale_exit_node == "dcl-platform-exit"


def test_headscale_exit_node_off_disables_guest_auto_apply(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.setenv("HEADSCALE_EXIT_NODE", "off")
    assert Settings.from_env().headscale_exit_node is None


def test_renterd_s3_env_is_required(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.delenv("RENTERD_S3_SECRET_KEY", raising=False)
    try:
        Settings.from_env()
        raise AssertionError("expected S3_SECRET_KEY to fail fast")
    except RuntimeError as exc:
        assert "S3_SECRET_KEY is required" in str(exc)


def test_s3_primary_env_names_are_accepted(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.delenv("RENTERD_S3_ENDPOINT", raising=False)
    monkeypatch.delenv("RENTERD_S3_ACCESS_KEY", raising=False)
    monkeypatch.delenv("RENTERD_S3_SECRET_KEY", raising=False)
    monkeypatch.delenv("RENTERD_S3_REGION", raising=False)
    monkeypatch.setenv("S3_ENDPOINT", "http://s3.primary.test.invalid")
    monkeypatch.setenv("S3_ACCESS_KEY", "primary-access-key")
    monkeypatch.setenv("S3_SECRET_KEY", "primary-secret-key")
    monkeypatch.setenv("S3_REGION", "eu-west-1")
    settings = Settings.from_env()
    assert settings.renterd_s3_endpoint == "http://s3.primary.test.invalid"
    assert settings.renterd_s3_region == "eu-west-1"
    dumped = repr(settings)
    assert "primary-secret-key" not in dumped
    assert "primary-access-key" not in dumped


def test_renterd_s3_endpoint_must_be_http(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.setenv("RENTERD_S3_ENDPOINT", "not-a-url")
    try:
        Settings.from_env()
        raise AssertionError("expected RENTERD_S3_ENDPOINT to fail fast")
    except RuntimeError as exc:
        assert "RENTERD_S3_ENDPOINT must be an http or https URL" in str(exc)


def test_renterd_s3_keys_must_be_at_least_16_chars(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.setenv("RENTERD_S3_ACCESS_KEY", "short")
    try:
        Settings.from_env()
        raise AssertionError("expected RENTERD_S3_ACCESS_KEY to fail fast")
    except RuntimeError as exc:
        assert "RENTERD_S3_ACCESS_KEY must be at least 16 characters" in str(exc)


def test_renterd_s3_region_defaults_to_us_east_1(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.delenv("RENTERD_S3_REGION", raising=False)
    settings = Settings.from_env()
    assert settings.renterd_s3_region == "us-east-1"


def test_renterd_guest_endpoint_defaults_to_s3_endpoint(monkeypatch) -> None:
    _required_env(monkeypatch)
    monkeypatch.setenv("CREDITS_PER_MINUTE", "1")
    monkeypatch.setenv("RENTERD_S3_GUEST_ENDPOINT", "http://guest.renterd.test.invalid")
    settings = Settings.from_env()
    assert settings.renterd_s3_guest_endpoint == "http://guest.renterd.test.invalid"
