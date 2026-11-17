"""Per-host credentials.

`HOST_BOOTSTRAP_TOKEN` is shared by every Host Agent, so on its own it lets
one host act as another, and lets any host command any agent. Each agent
therefore generates a random secret on first run and sends it in the
`X-DCL-Host-Secret` header:

- Host Agent -> Matchmaker: the header must match the secret this host
  registered with. The Matchmaker stores only `auth_hash(secret)`.
- Matchmaker -> Host Agent (`/v1/provision`, `/v1/teardown`): the Bearer is
  `command_token(secret)`. The agent derives the same value and refuses
  anything else, so another host's token cannot command it.

The secret is first-use: whoever registers a host id sets its secret, and
only that secret can change the host afterwards. Hosts registered before
this existed have no secret and are still accepted on the bootstrap token
alone until an agent sends one.
"""

from __future__ import annotations

import hashlib
import hmac

HOST_SECRET_HEADER = "X-DCL-Host-Secret"
MIN_HOST_SECRET_CHARS = 32
MAX_HOST_SECRET_CHARS = 256

_AUTH_DOMAIN = "dcl-host-auth:"
_COMMAND_DOMAIN = "dcl-agent-command:"


def _digest(domain: str, secret: str) -> str:
    return hashlib.sha256((domain + secret).encode("utf-8")).hexdigest()


def auth_hash(secret: str) -> str:
    """What the Matchmaker stores to recognise a host. Not the command token."""
    return _digest(_AUTH_DOMAIN, secret)


def command_token(secret: str) -> str:
    """Bearer the Matchmaker presents to this host's agent."""
    return _digest(_COMMAND_DOMAIN, secret)


def secret_is_well_formed(secret: str) -> bool:
    return (
        MIN_HOST_SECRET_CHARS <= len(secret) <= MAX_HOST_SECRET_CHARS
        and secret.isascii()
        and secret.isprintable()
        and " " not in secret
    )


def secret_matches(secret: str, stored_hash: str) -> bool:
    return hmac.compare_digest(auth_hash(secret), stored_hash)
