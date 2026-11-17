"""Sia renterd S3-compatible client (path-style).

Architecture §8: persistent files live off the volunteer host. Matchmaker
talks to a self-hosted renterd S3 gateway, records a per-user bucket, and
returns mount info for the **guest**. Decryption/session keys must not be
written to the host OS disk. rclone/s3 mount belongs inside the guest.

Unit tests mock HTTP. Never point pytest at a live Sia network.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from hashlib import sha256
from typing import Protocol
from urllib.parse import quote, urlparse
from xml.etree import ElementTree
import hmac

import httpx

_REQUEST_TIMEOUT_SECONDS = 10.0
_SERVICE = "s3"
_MAX_LIST_PAGES = 200


class RenterdError(Exception):
    """renterd S3 call failed. Message must not include access or secret keys."""


@dataclass(frozen=True)
class StorageGrant:
    """Guest-side S3 locators. Credentials stay in Matchmaker/guest env, not JSON."""

    endpoint: str
    region: str
    bucket: str
    prefix: str
    path_style: bool
    mount_inside_guest: bool
    rclone: str
    objects: tuple[str, ...]


class RenterdClient(Protocol):
    def create_bucket(self, name: str) -> bool:
        """Create `name`. True if created, False if it already existed."""

    def put_object(self, bucket: str, key: str, body: bytes) -> None:
        """PUT an object (path-style)."""

    def list_object_keys(self, bucket: str, prefix: str) -> list[str]:
        """List object keys under `prefix` (path-style ListObjectsV2)."""

    def usage_bytes(self, bucket: str, prefix: str, *, stop_at: int | None = None) -> int:
        """Total size of objects under `prefix`. May stop early past `stop_at`."""


class HttpRenterdClient:
    """Path-style S3 against renterd (`RENTERD_S3_ENDPOINT`). SigV4.

    Redirects are refused so credentials are not replayed against another host.
    """

    def __init__(
        self,
        *,
        endpoint: str,
        access_key: str,
        secret_key: str,
        region: str,
        timeout_seconds: float = _REQUEST_TIMEOUT_SECONDS,
        transport: httpx.BaseTransport | None = None,
        session_token: str | None = None,
    ) -> None:
        self._session_token = session_token
        self._endpoint = endpoint.rstrip("/")
        self._access_key = access_key
        self._secret_key = secret_key
        self._region = region
        self._timeout_seconds = timeout_seconds
        self._transport = transport

    def create_bucket(self, name: str) -> bool:
        status, _ = self._request("PUT", f"/{quote(name, safe='-')}", body=b"")
        if status in {200, 201, 204}:
            return True
        if status == 409:
            return False
        raise RenterdError(f"renterd rejected CreateBucket (HTTP {status})")

    def put_object(self, bucket: str, key: str, body: bytes) -> None:
        encoded_key = quote(key, safe="/-._~")
        path = f"/{quote(bucket, safe='-')}/{encoded_key}"
        status, _ = self._request("PUT", path, body=body)
        if status not in {200, 201, 204}:
            raise RenterdError(f"renterd rejected PutObject (HTTP {status})")

    def list_object_keys(self, bucket: str, prefix: str) -> list[str]:
        query = _canonical_query({"list-type": "2", "prefix": prefix})
        path = f"/{quote(bucket, safe='-')}?{query}"
        status, body = self._request("GET", path, body=b"")
        if status != 200:
            raise RenterdError(f"renterd rejected ListObjectsV2 (HTTP {status})")
        return _keys_from_list_xml(body)

    def usage_bytes(self, bucket: str, prefix: str, *, stop_at: int | None = None) -> int:
        total = 0
        token: str | None = None
        for _ in range(_MAX_LIST_PAGES):
            params = {"list-type": "2", "prefix": prefix}
            if token:
                params["continuation-token"] = token
            path = f"/{quote(bucket, safe='-')}?{_canonical_query(params)}"
            status, body = self._request("GET", path, body=b"")
            if status != 200:
                raise RenterdError(f"S3 rejected ListObjectsV2 (HTTP {status})")
            sizes, token = _sizes_from_list_xml(body)
            total += sum(sizes)
            if token is None or (stop_at is not None and total >= stop_at):
                break
        return total

    def _request(self, method: str, path: str, *, body: bytes) -> tuple[int, bytes]:
        url = f"{self._endpoint}{path}"
        headers = _sigv4_headers(
            method=method,
            url=url,
            body=body,
            access_key=self._access_key,
            secret_key=self._secret_key,
            region=self._region,
            session_token=self._session_token,
        )
        try:
            with httpx.Client(
                timeout=self._timeout_seconds,
                follow_redirects=False,
                transport=self._transport,
            ) as client:
                response = client.request(method, url, content=body, headers=headers)
        except httpx.HTTPError as exc:
            raise RenterdError("could not reach renterd S3") from exc
        return response.status_code, response.content


def _sigv4_headers(
    *,
    method: str,
    url: str,
    body: bytes,
    access_key: str,
    secret_key: str,
    region: str,
    now: datetime | None = None,
    session_token: str | None = None,
) -> dict[str, str]:
    moment = now if now is not None else datetime.now(timezone.utc)
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=timezone.utc)
    moment = moment.astimezone(timezone.utc)
    amz_date = moment.strftime("%Y%m%dT%H%M%SZ")
    date_stamp = moment.strftime("%Y%m%d")
    parsed = urlparse(url)
    host = parsed.netloc
    canonical_uri = parsed.path or "/"
    canonical_querystring = parsed.query
    payload_hash = sha256(body).hexdigest()
    canonical_headers = (
        f"host:{host}\n"
        f"x-amz-content-sha256:{payload_hash}\n"
        f"x-amz-date:{amz_date}\n"
    )
    signed_headers = "host;x-amz-content-sha256;x-amz-date"
    if session_token:
        # Temporary credentials: the token is a signed header.
        canonical_headers += f"x-amz-security-token:{session_token}\n"
        signed_headers += ";x-amz-security-token"
    canonical_request = "\n".join(
        [
            method,
            canonical_uri,
            canonical_querystring,
            canonical_headers,
            signed_headers,
            payload_hash,
        ]
    )
    algorithm = "AWS4-HMAC-SHA256"
    credential_scope = f"{date_stamp}/{region}/{_SERVICE}/aws4_request"
    string_to_sign = "\n".join(
        [
            algorithm,
            amz_date,
            credential_scope,
            sha256(canonical_request.encode("utf-8")).hexdigest(),
        ]
    )
    signing_key = _signing_key(secret_key, date_stamp, region)
    signature = hmac.new(
        signing_key, string_to_sign.encode("utf-8"), sha256
    ).hexdigest()
    authorization = (
        f"{algorithm} Credential={access_key}/{credential_scope}, "
        f"SignedHeaders={signed_headers}, Signature={signature}"
    )
    headers = {
        "Host": host,
        "x-amz-content-sha256": payload_hash,
        "x-amz-date": amz_date,
        "Authorization": authorization,
    }
    if session_token:
        headers["x-amz-security-token"] = session_token
    return headers


def _signing_key(secret_key: str, date_stamp: str, region: str) -> bytes:
    date_key = hmac.new(
        f"AWS4{secret_key}".encode("utf-8"), date_stamp.encode("utf-8"), sha256
    ).digest()
    region_key = hmac.new(date_key, region.encode("utf-8"), sha256).digest()
    service_key = hmac.new(region_key, _SERVICE.encode("utf-8"), sha256).digest()
    return hmac.new(service_key, b"aws4_request", sha256).digest()


def _canonical_query(params: dict[str, str]) -> str:
    return "&".join(
        f"{quote(key, safe='-_.~')}={quote(value, safe='-_.~')}"
        for key, value in sorted(params.items())
    )


def _keys_from_list_xml(body: bytes) -> list[str]:
    try:
        root = ElementTree.fromstring(body)
    except ElementTree.ParseError as exc:
        raise RenterdError("renterd ListObjectsV2 XML was invalid") from exc
    keys: list[str] = []
    for element in root.iter():
        if element.tag.endswith("Key") and element.text:
            keys.append(element.text)
    return keys


def _sizes_from_list_xml(body: bytes) -> tuple[list[int], str | None]:
    """Object sizes on one ListObjectsV2 page, and the next page token if any."""
    try:
        root = ElementTree.fromstring(body)
    except ElementTree.ParseError as exc:
        raise RenterdError("S3 ListObjectsV2 XML was invalid") from exc
    sizes: list[int] = []
    token: str | None = None
    for element in root.iter():
        tag = element.tag.rsplit("}", 1)[-1]
        if tag == "Size" and element.text and element.text.strip().isdigit():
            sizes.append(int(element.text.strip()))
        elif tag == "NextContinuationToken" and element.text:
            token = element.text
    return sizes, token
