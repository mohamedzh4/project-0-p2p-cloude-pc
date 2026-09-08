from __future__ import annotations

import hmac
from typing import Annotated
from uuid import UUID

from fastapi import Depends, HTTPException, status
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.auth import decode_access_token
from matchmaker.config import get_settings
from matchmaker.db import get_db
from matchmaker.headscale import HeadscaleClient, HttpHeadscaleClient
from matchmaker.images import registry_from_settings
from matchmaker.matching import HostSelector, RankedHostSelector
from matchmaker.models import User
from matchmaker.provision import HostProvisioner, HttpHostProvisioner
from matchmaker.renterd import HttpRenterdClient, RenterdClient

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
    settings = get_settings()
    return HttpHostProvisioner(
        fallback_base_url=settings.host_agent_provision_url,
        bearer_token=settings.host_bootstrap_token,
        images=registry_from_settings(settings),
        storage_endpoint=settings.renterd_s3_guest_endpoint,
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


def require_host_bootstrap(
    credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(bearer_scheme)],
) -> None:
    _require_static_bearer(credentials, get_settings().host_bootstrap_token)


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
