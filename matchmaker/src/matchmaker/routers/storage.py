"""Per-user renterd bucket + guest rclone locators (FR4)."""

from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, Response, status
from sqlalchemy.orm import Session

from matchmaker.config import get_settings
from matchmaker.db import get_db
from matchmaker.deps import get_current_user, get_renterd_client
from matchmaker.models import User, UserStorage
from matchmaker.renterd import RenterdClient, RenterdError
from matchmaker.schemas import StorageMountInfo
from matchmaker.storage import (
    ensure_user_storage,
    grant_from_row,
    mount_info_from_grant,
)

router = APIRouter()


def _provision(db: Session, user: User, renterd: RenterdClient) -> StorageMountInfo:
    settings = get_settings()
    try:
        grant = ensure_user_storage(
            db,
            user,
            renterd,
            endpoint=settings.renterd_s3_guest_endpoint,
            region=settings.renterd_s3_region,
        )
    except RenterdError as exc:
        raise HTTPException(
            status_code=status.HTTP_502_BAD_GATEWAY,
            detail="renterd storage provision failed",
        ) from exc
    return mount_info_from_grant(grant)


@router.post("/storage", response_model=StorageMountInfo)
def post_user_storage(
    response: Response,
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
    renterd: Annotated[RenterdClient, Depends(get_renterd_client)],
) -> StorageMountInfo:
    """Issue (or reuse) this user's renterd bucket. Mount inside the guest."""
    existed = db.get(UserStorage, user.id) is not None
    body = _provision(db, user, renterd)
    response.status_code = status.HTTP_200_OK if existed else status.HTTP_201_CREATED
    return body


@router.get("/storage", response_model=StorageMountInfo)
def get_user_storage(
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
    renterd: Annotated[RenterdClient, Depends(get_renterd_client)],
) -> StorageMountInfo:
    """List this user's bucket metadata. 404 if never issued."""
    row = db.get(UserStorage, user.id)
    if row is None:
        raise HTTPException(
            status_code=status.HTTP_404_NOT_FOUND,
            detail="Storage not found",
        )
    settings = get_settings()
    try:
        grant = grant_from_row(
            row,
            renterd,
            endpoint=settings.renterd_s3_guest_endpoint,
            region=settings.renterd_s3_region,
        )
    except RenterdError as exc:
        raise HTTPException(
            status_code=status.HTTP_502_BAD_GATEWAY,
            detail="renterd storage provision failed",
        ) from exc
    return mount_info_from_grant(grant)
