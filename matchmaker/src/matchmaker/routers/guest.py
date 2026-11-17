"""Guest-only Matchmaker routes. Auth is the bootstrap token, not a user JWT."""

from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, status
from fastapi.security import HTTPAuthorizationCredentials
from sqlalchemy.orm import Session

from matchmaker.config import get_settings
from matchmaker.db import get_db
from matchmaker.deps import bearer_scheme, get_renterd_client, get_storage_credential_issuer
from matchmaker.renterd import RenterdClient, RenterdError
from matchmaker.storage_credentials import StorageCredentialError, StorageCredentialIssuer
from matchmaker.guest_bootstrap import (
    bootstrap_response,
    lookup_guest_bootstrap,
    record_saved_as_of,
)
from matchmaker.schemas import (
    GuestBootstrapResponse,
    GuestSavedAsOfRequest,
    GuestSavedAsOfResponse,
)

router = APIRouter()


def _require_bootstrap_row(
    credentials: HTTPAuthorizationCredentials | None,
    db: Session,
):
    if credentials is None or credentials.scheme.lower() != "bearer":
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Not authenticated",
        )
    row = lookup_guest_bootstrap(db, credentials.credentials)
    if row is None:
        raise HTTPException(
            status_code=status.HTTP_404_NOT_FOUND,
            detail="Session not found",
        )
    return row


@router.post("/guest/bootstrap", response_model=GuestBootstrapResponse)
def redeem_guest_bootstrap(
    db: Annotated[Session, Depends(get_db)],
    credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(bearer_scheme)],
    issuer: Annotated[StorageCredentialIssuer, Depends(get_storage_credential_issuer)],
    store: Annotated[RenterdClient, Depends(get_renterd_client)],
) -> GuestBootstrapResponse:
    """Hand the guest its mesh key and a storage key for this one user."""
    row = _require_bootstrap_row(credentials, db)
    settings = get_settings()
    quota = settings.storage_quota_bytes
    try:
        used = store.usage_bytes(row.storage_bucket, row.storage_prefix, stop_at=quota)
    except RenterdError:
        # Unknown usage must not lock a user out of their files.
        used = 0
    read_only = used >= quota
    try:
        issued = issuer.issue(
            bucket=row.storage_bucket,
            prefix=row.storage_prefix,
            read_only=read_only,
            ttl_seconds=settings.storage_credential_ttl_seconds,
        )
    except StorageCredentialError as exc:
        raise HTTPException(
            status_code=status.HTTP_502_BAD_GATEWAY,
            detail="storage credentials could not be issued",
        ) from exc
    return bootstrap_response(row, credentials=issued, read_only=read_only)


@router.post("/guest/saved-as-of", response_model=GuestSavedAsOfResponse)
def report_saved_as_of(
    db: Annotated[Session, Depends(get_db)],
    credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(bearer_scheme)],
    payload: GuestSavedAsOfRequest | None = None,
) -> GuestSavedAsOfResponse:
    row = _require_bootstrap_row(credentials, db)
    body = payload or GuestSavedAsOfRequest()
    saved = record_saved_as_of(db, row, saved_as_of=body.saved_as_of)
    return GuestSavedAsOfResponse(session_id=row.session_id, saved_as_of=saved)
