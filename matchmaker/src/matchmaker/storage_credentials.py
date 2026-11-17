"""Storage credentials handed to one guest.

A guest runs on a volunteer's PC, and the volunteer can read its memory.
So a guest must only ever hold a key for the files of the one user it
serves. Three issuers:

- `R2LocalCredentialIssuer` (production default): makes Cloudflare R2
  temporary credentials on this server by signing a short token with the
  parent R2 key. Needs no Cloudflare API token and no network call.
- `R2TempCredentialIssuer`: asks the Cloudflare API for the same thing.
  Used when `R2_API_TOKEN` is set.

  Both are limited to one bucket and one prefix. They expire on their own;
  R2 has no way to revoke a single one early, so the lifetime is kept to
  the longest allowed session.
- `StaticCredentialIssuer` (local development only): returns the
  Matchmaker's own S3 key, which opens every user's files. Never use it
  with real volunteers.
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import json
import time
from dataclasses import dataclass, field
from typing import Protocol
from urllib.parse import urlparse

import httpx

_CLOUDFLARE_API = "https://api.cloudflare.com/client/v4"
_REQUEST_TIMEOUT_SECONDS = 10.0
R2_MAX_TTL_SECONDS = 604800

MODE_STATIC = "static"
MODE_R2 = "r2"


class StorageCredentialError(Exception):
    """Could not issue guest credentials. Message must not include any key."""


@dataclass(frozen=True)
class GuestStorageCredentials:
    access_key_id: str
    secret_access_key: str = field(repr=False)
    session_token: str | None = field(default=None, repr=False)


class StorageCredentialIssuer(Protocol):
    def issue(
        self, *, bucket: str, prefix: str, read_only: bool, ttl_seconds: int
    ) -> GuestStorageCredentials:
        """Credentials for objects under `prefix` in `bucket`, and nothing else."""


class StaticCredentialIssuer:
    """Development only: the master key, with no per-user limit."""

    scoped = False

    def __init__(self, *, access_key: str, secret_key: str) -> None:
        self._access_key = access_key
        self._secret_key = secret_key

    def issue(
        self, *, bucket: str, prefix: str, read_only: bool, ttl_seconds: int
    ) -> GuestStorageCredentials:
        return GuestStorageCredentials(
            access_key_id=self._access_key,
            secret_access_key=self._secret_key,
        )


class R2TempCredentialIssuer:
    """`POST /accounts/{id}/r2/temp-access-credentials` on the Cloudflare API."""

    scoped = True

    def __init__(
        self,
        *,
        account_id: str,
        api_token: str,
        parent_access_key_id: str,
        api_base: str = _CLOUDFLARE_API,
        timeout_seconds: float = _REQUEST_TIMEOUT_SECONDS,
        transport: httpx.BaseTransport | None = None,
    ) -> None:
        self._url = f"{api_base.rstrip('/')}/accounts/{account_id}/r2/temp-access-credentials"
        self._api_token = api_token
        self._parent_access_key_id = parent_access_key_id
        self._timeout_seconds = timeout_seconds
        self._transport = transport

    def issue(
        self, *, bucket: str, prefix: str, read_only: bool, ttl_seconds: int
    ) -> GuestStorageCredentials:
        if not prefix.strip().strip("/"):
            # An empty prefix would open the whole bucket.
            raise StorageCredentialError("refusing to issue credentials without a prefix")
        body = {
            "bucket": bucket,
            "parentAccessKeyId": self._parent_access_key_id,
            "permission": "object-read-only" if read_only else "object-read-write",
            "ttlSeconds": max(60, min(int(ttl_seconds), R2_MAX_TTL_SECONDS)),
            "prefixes": [prefix],
        }
        headers = {
            "Authorization": f"Bearer {self._api_token}",
            "Content-Type": "application/json",
        }
        try:
            with httpx.Client(
                timeout=self._timeout_seconds,
                follow_redirects=False,
                transport=self._transport,
            ) as client:
                response = client.post(self._url, json=body, headers=headers)
        except httpx.HTTPError as exc:
            raise StorageCredentialError("could not reach the Cloudflare API") from exc
        if response.status_code != 200:
            raise StorageCredentialError(
                f"Cloudflare rejected the temporary credential request (HTTP {response.status_code})"
            )
        try:
            result = response.json().get("result")
        except (ValueError, AttributeError) as exc:
            raise StorageCredentialError("Cloudflare returned an unreadable response") from exc
        if not isinstance(result, dict):
            raise StorageCredentialError("Cloudflare returned no credentials")
        access = result.get("accessKeyId")
        secret = result.get("secretAccessKey")
        token = result.get("sessionToken")
        if not all(isinstance(v, str) and v for v in (access, secret, token)):
            raise StorageCredentialError("Cloudflare returned incomplete credentials")
        return GuestStorageCredentials(
            access_key_id=access,
            secret_access_key=secret,
            session_token=token,
        )


def _b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode("ascii")


class R2LocalCredentialIssuer:
    """R2 temporary credentials by local signing (no Cloudflare API call).

    Cloudflare's documented scheme: sign a JWT (HS256) with the parent
    secret access key; the temporary secret is the SHA-256 hex digest of
    that JWT; the session token is base64("jwt/" + JWT); the access key id
    is the parent's. R2 checks the signature and enforces bucket, scope,
    paths and expiry on every request.
    """

    scoped = True

    def __init__(
        self,
        *,
        account_id: str,
        endpoint: str,
        parent_access_key_id: str,
        parent_secret_access_key: str,
    ) -> None:
        host = urlparse(endpoint).netloc
        if not host:
            raise StorageCredentialError("the R2 endpoint has no host")
        self._account_id = account_id
        self._audience = host
        self._parent_access_key_id = parent_access_key_id
        self._parent_secret = parent_secret_access_key.encode("utf-8")

    def issue(
        self,
        *,
        bucket: str,
        prefix: str,
        read_only: bool,
        ttl_seconds: int,
        now: int | None = None,
    ) -> GuestStorageCredentials:
        if not prefix.strip().strip("/"):
            # An empty prefix would open the whole bucket.
            raise StorageCredentialError("refusing to issue credentials without a prefix")
        issued_at = int(time.time()) if now is None else now
        ttl = max(60, min(int(ttl_seconds), R2_MAX_TTL_SECONDS))
        header = {"alg": "HS256", "typ": "JWT"}
        claims = {
            "bucket": bucket,
            "scope": "object-read-only" if read_only else "object-read-write",
            "paths": {"prefixPaths": [prefix], "objectPaths": []},
            "sub": self._account_id,
            "iss": self._parent_access_key_id,
            "aud": self._audience,
            "iat": issued_at,
            "exp": issued_at + ttl,
        }
        signing_input = (
            _b64url(json.dumps(header, separators=(",", ":")).encode())
            + "."
            + _b64url(json.dumps(claims, separators=(",", ":")).encode())
        )
        signature = hmac.new(
            self._parent_secret, signing_input.encode("ascii"), hashlib.sha256
        ).digest()
        token = f"{signing_input}.{_b64url(signature)}"
        return GuestStorageCredentials(
            access_key_id=self._parent_access_key_id,
            secret_access_key=hashlib.sha256(token.encode("ascii")).hexdigest(),
            session_token=base64.b64encode(f"jwt/{token}".encode("ascii")).decode("ascii"),
        )
