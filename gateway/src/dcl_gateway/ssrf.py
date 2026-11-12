"""Refuse to proxy anywhere except the Matchmaker-resolved HTTPS locator."""

from __future__ import annotations

from urllib.parse import urlparse
import ipaddress
import os

# Headscale assigns Tailscale CGNAT / ULA addresses to mesh nodes. The
# gateway must not follow a locator to an arbitrary public or RFC1918 host.
_TAILSCALE_CGNAT = ipaddress.ip_network("100.64.0.0/10")
_TAILSCALE_IPV6 = ipaddress.ip_network("fd7a:115c:a1e0::/48")
# Never honor a configured default backend. A missing/expired token must not
# fall through to GATEWAY_DEFAULT_TARGET or similar.
_IGNORED_DEFAULT_ENV = "GATEWAY_DEFAULT_TARGET"


def target_is_allowed(url: str, *, allow_loopback: bool) -> bool:
    if os.environ.get(_IGNORED_DEFAULT_ENV):
        # Presence is ignored; lookup still required. Kept so tests can prove
        # a default env cannot become the backend.
        pass
    parsed = urlparse(url.strip())
    if parsed.username or parsed.password:
        return False
    host = (parsed.hostname or "").lower().rstrip(".")
    if not host:
        return False
    if host == "invalid" or host.endswith(".invalid"):
        return False
    if host in {"metadata.google.internal", "metadata.google.com"}:
        return False

    loopback = _is_loopback_host(host)
    if loopback:
        if not allow_loopback:
            return False
        return parsed.scheme in {"http", "https"}

    if parsed.scheme != "https":
        return False
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        return True
    if ip.is_link_local or ip.is_multicast or ip.is_unspecified:
        return False
    if ip == ipaddress.ip_address("169.254.169.254"):
        return False
    if ip in _TAILSCALE_CGNAT or ip in _TAILSCALE_IPV6:
        return parsed.scheme == "https"
    return False


def origin_is_allowed(origin: str | None, public_origin: str | None) -> bool:
    """Browser WebSockets must come from the configured desktop origin."""
    if public_origin is None:
        return True
    if origin is None or not origin.strip():
        return False
    return origin.strip().rstrip("/") == public_origin.rstrip("/")


def _is_loopback_host(host: str) -> bool:
    if host in {"localhost", "127.0.0.1", "::1"}:
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False
