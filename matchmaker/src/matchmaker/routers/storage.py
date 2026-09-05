"""Per-user renterd bucket + guest rclone grant (FR4)."""

from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, status
from sqlalchemy.orm import Session

from matchmaker.config import get_settings
from matchmaker.db import get_db
from matchmaker.deps import get_current_user, get_renterd_client
from matchmaker.models import User
from matchmaker.renterd import RenterdClient, RenterdError
from matchmaker.schemas import StorageMountInfo
from matchmaker.storage import ensure_user_storage, mount_info_from_grant

router = APIRouter()


@router.get("/storage", response_model=StorageMountInfo)
def get_user_storage(
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
    renterd: Annotated[RenterdClient, Depends(get_renterd_client)],
) -> StorageMountInfo:
    """Provision (or reuse) this user's renterd bucket. Mount inside the guest."""
    settings = get_settings()
    try:
        grant = ensure_user_storage(
            db,
            user,
            renterd,
            endpoint=settings.renterd_s3_guest_endpoint,
            region=settings.renterd_s3_region,
            access_key_id=settings.renterd_s3_access_key,
            secret_access_key=settings.renterd_s3_secret_key,
        )
    except RenterdError as exc:
        raise HTTPException(
            status_code=status.HTTP_502_BAD_GATEWAY,
            detail="renterd storage provision failed",
        ) from exc
    return mount_info_from_grant(grant)
