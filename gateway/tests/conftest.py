from __future__ import annotations

import os

os.environ.setdefault("MATCHMAKER_URL", "http://matchmaker.test.invalid")
os.environ.setdefault("GATEWAY_RESOLVE_TOKEN", "test-gateway-resolve-token")
os.environ.setdefault("GATEWAY_ALLOW_LOOPBACK_TARGET", "1")
os.environ.setdefault("GATEWAY_FAIL_LIMIT", "5")
os.environ.setdefault("GATEWAY_FAIL_WINDOW_SECONDS", "60")
os.environ.pop("GATEWAY_PUBLIC_ORIGIN", None)

from dcl_gateway.config import get_settings  # noqa: E402

get_settings.cache_clear()
