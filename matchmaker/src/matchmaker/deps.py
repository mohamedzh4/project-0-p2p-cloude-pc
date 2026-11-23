from __future__ import annotations

import hmac
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Annotated
from uuid import UUID

from fastapi import Depends, Header, HTTPException, status
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.auth import decode_access_token
from matchmaker.booking import ProvisionScheduler, ThreadScheduler
from matchmaker.config import get_settings
from matchmaker.db import get_db
from matchmaker.headscale import HeadscaleClient, HttpHeadscaleClient
from matchmaker.images import registry_from_settings
from matchmaker.matching import HostSelector, RankedHostSelector
from matchmaker.host_credentials import (
    HOST_SECRET_HEADER,
    auth_hash,
    command_token,
    secret_is_well_formed,
    secret_matches,
)
from matchmaker.host_kits import find_code as find_enrollment_code
from matchmaker.models import Host, HostEnrollmentCode, User
from matchmaker.provision import HostProvisioner, HttpHostProvisioner
from matchmaker.renterd import HttpRenterdClient, RenterdClient
from matchmaker.storage_credentials import (
    MODE_R2,
    R2LocalCredentialIssuer,
    R2TempCredentialIssuer,
    StaticCredentialIssuer,
    StorageCredentialIssuer,
)

bearer_scheme = HTTPBearer(auto_error=False)


def _require_static_bearer(
    credentials: HTTPAuthorizationCredentials | None,
    expected: str,
) -> None:
    if credentials is None or credentials.scheme.lower() != "bearer":
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Not authenticated",
        )
    if not hmac.compare_digest(credentials.credentials, expected):
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Not authenticated",
        )


def get_current_user(
    credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(bearer_scheme)],
    db: Annotated[Session, Depends(get_db)],
) -> User:
    if credentials is None or credentials.scheme.lower() != "bearer":
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Not authenticated",
        )
    try:
        user_id: UUID = decode_access_token(credentials.credentials)
    except ValueError:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Not authenticated",
        ) from None

    user = db.scalars(select(User).where(User.id == user_id)).first()
    if user is None:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Not authenticated",
        )
    return user


def get_host_selector() -> HostSelector:
    return RankedHostSelector()


def get_host_provisioner() -> HostProvisioner:
    """Build the provisioner. Also called outside a request (sweep, jobs).

    Imported here, not at the top: host_commands needs the database module,
    which tests replace before the app is built.
    """
    from matchmaker.db import get_session_factory
    from matchmaker.host_commands import RoutingHostProvisioner

    settings = get_settings()
    http = HttpHostProvisioner(
        fallback_base_url=settings.host_agent_provision_url,
        bearer_token=settings.host_bootstrap_token,
        images=registry_from_settings(settings),
        storage_endpoint=settings.renterd_s3_guest_endpoint,
    )
    return RoutingHostProvisioner(http, get_session_factory())


_thread_scheduler: ThreadScheduler | None = None


def get_provision_scheduler() -> ProvisionScheduler:
    """One shared pool for the slow agent calls."""
    global _thread_scheduler
    if _thread_scheduler is None:
        _thread_scheduler = ThreadScheduler()
    return _thread_scheduler


def get_storage_credential_issuer() -> StorageCredentialIssuer:
    settings = get_settings()
    if settings.storage_credentials_mode == MODE_R2 and not settings.r2_api_token:
        return R2LocalCredentialIssuer(
            account_id=settings.r2_account_id or "",
            endpoint=settings.renterd_s3_endpoint,
            parent_access_key_id=settings.renterd_s3_access_key,
            parent_secret_access_key=settings.renterd_s3_secret_key,
        )
    if settings.storage_credentials_mode == MODE_R2:
        return R2TempCredentialIssuer(
            account_id=settings.r2_account_id or "",
            api_token=settings.r2_api_token or "",
            parent_access_key_id=settings.renterd_s3_access_key,
            **({"api_base": settings.r2_api_base} if settings.r2_api_base else {}),
        )
    return StaticCredentialIssuer(
        access_key=settings.renterd_s3_access_key,
        secret_key=settings.renterd_s3_secret_key,
    )


def get_headscale_client() -> HeadscaleClient:
    settings = get_settings()
    return HttpHeadscaleClient(
        base_url=settings.headscale_url,
        api_key=settings.headscale_api_key,
        user=settings.headscale_user,
        login_server=settings.headscale_login_server,
    )


def get_renterd_client() -> RenterdClient:
    settings = get_settings()
    return HttpRenterdClient(
        endpoint=settings.renterd_s3_endpoint,
        access_key=settings.renterd_s3_access_key,
        secret_key=settings.renterd_s3_secret_key,
        region=settings.renterd_s3_region,
    )


@dataclass(frozen=True)
class HostBearer:
    """Who sent a host request: the shared token (`code` None) or the
    registration code of a kit downloaded from the website."""

    code: HostEnrollmentCode | None = None


def require_host_bootstrap(
    credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(bearer_scheme)],
    db: Annotated[Session, Depends(get_db)],
) -> HostBearer:
    if credentials is None or credentials.scheme.lower() != "bearer":
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Not authenticated")
    if hmac.compare_digest(credentials.credentials, get_settings().host_bootstrap_token):
        return HostBearer()
    code = find_enrollment_code(db, credentials.credentials)
    if code is None:
        raise HTTPException(status_code=status.HTTP_401_UNAUTHORIZED, detail="Not authenticated")
    return HostBearer(code=code)


def bind_enrollment_code(bearer: HostBearer, host: Host) -> None:
    """A kit's code registers one new host, then belongs to it."""
    code = bearer.code
    if code is None:
        return
    if code.host_id is not None:
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="This registration code was already used by another PC",
        )
    code.host_id = host.id
    code.used_at = datetime.now(timezone.utc)
    host.owner_user_id = code.user_id


def check_enrollment_code(bearer: HostBearer, host: Host) -> None:
    if bearer.code is not None and bearer.code.host_id != host.id:
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Host credential does not match this host",
        )


def host_secret_header(
    secret: Annotated[str | None, Header(alias=HOST_SECRET_HEADER)] = None,
) -> str | None:
    """The agent's own secret, when it sent one. Malformed values are refused."""
    if secret is None:
        return None
    if not secret_is_well_formed(secret):
        raise HTTPException(
            status_code=status.HTTP_400_BAD_REQUEST,
            detail=f"{HOST_SECRET_HEADER} is malformed",
        )
    return secret


def claim_host_secret(host: Host, secret: str | None) -> None:
    """First use: record the secret a host registers with. Never overwrites."""
    if secret is None or host.secret_hash is not None:
        return
    host.secret_hash = auth_hash(secret)
    host.command_token = command_token(secret)


def check_host_secret(host: Host, secret: str | None) -> None:
    """A host that has a secret is only reachable with that secret."""
    if host.secret_hash is None:
        return
    if secret is None or not secret_matches(secret, host.secret_hash):
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN,
            detail="Host credential does not match this host",
        )


def get_authorized_host(
    host_id: UUID,
    db: Annotated[Session, Depends(get_db)],
    bearer: Annotated[HostBearer, Depends(require_host_bootstrap)],
    secret: Annotated[str | None, Depends(host_secret_header)],
) -> Host:
    """Bootstrap Bearer, then this host's own secret. 404 for an unknown id."""
    host = db.get(Host, host_id)
    if host is None:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Host not found")
    check_enrollment_code(bearer, host)
    check_host_secret(host, secret)
    return host


def require_host_approver(
    credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(bearer_scheme)],
) -> None:
    _require_static_bearer(credentials, get_settings().host_approve_token)


def require_gateway_resolver(
    credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(bearer_scheme)],
) -> None:
    expected = get_settings().gateway_resolve_token
    if expected is None:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Not authenticated",
        )
    _require_static_bearer(credentials, expected)
