"""Per-user renterd bucket records and guest rclone instructions.

Mount belongs **inside the guest**. Never rclone on the volunteer host OS.
Keys in the session JSON are for the guest process, not for host-disk cache.
"""

from __future__ import annotations

from datetime import datetime, timezone
from uuid import UUID

from sqlalchemy.orm import Session

from matchmaker.models import User, UserStorage
from matchmaker.renterd import RenterdClient, StorageGrant
from matchmaker.schemas import StorageMountInfo

USER_FILES_PREFIX = "files/"
PROVISION_MARKER_KEY = ".dcl/provisioned"


def bucket_name_for_user(user_id: UUID) -> str:
    """S3-legal per-user bucket. Hyphenated UUID, prefixed so it is not global."""
    return f"dcl-{user_id}"


def guest_rclone_mount_command(*, bucket: str, prefix: str, endpoint: str) -> str:
    """rclone argv for the guest. Credentials stay in env, not in `ps` argv."""
    remote = f":s3:{bucket}"
    trimmed = prefix.strip("/")
    if trimmed:
        remote = f"{remote}/{trimmed}"
    return (
        "rclone mount "
        f"{remote} /home/user/Persistent "
        "--s3-provider Other "
        f"--s3-endpoint {endpoint} "
        "--s3-env-auth "
        "--s3-force-path-style "
        "--vfs-cache-mode full"
    )


def storage_grant(
    *,
    user_id: UUID,
    endpoint: str,
    region: str,
    access_key_id: str,
    secret_access_key: str,
    prefix: str = USER_FILES_PREFIX,
) -> StorageGrant:
    bucket = bucket_name_for_user(user_id)
    return StorageGrant(
        endpoint=endpoint,
        region=region,
        bucket=bucket,
        prefix=prefix,
        path_style=True,
        access_key_id=access_key_id,
        secret_access_key=secret_access_key,
        mount_inside_guest=True,
        rclone=guest_rclone_mount_command(bucket=bucket, prefix=prefix, endpoint=endpoint),
    )


def ensure_user_storage(
    db: Session,
    user: User,
    renterd: RenterdClient,
    *,
    endpoint: str,
    region: str,
    access_key_id: str,
    secret_access_key: str,
) -> StorageGrant:
    """Create the per-user bucket if needed and record it. Idempotent."""
    grant = storage_grant(
        user_id=user.id,
        endpoint=endpoint,
        region=region,
        access_key_id=access_key_id,
        secret_access_key=secret_access_key,
    )
    created = renterd.create_bucket(grant.bucket)
    if created:
        marker = f"provisioned {datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')}\n"
        renterd.put_object(grant.bucket, PROVISION_MARKER_KEY, marker.encode("utf-8"))

    row = db.get(UserStorage, user.id)
    if row is None:
        db.add(
            UserStorage(
                user_id=user.id,
                bucket=grant.bucket,
                prefix=grant.prefix,
            )
        )
    else:
        row.bucket = grant.bucket
        row.prefix = grant.prefix
    db.commit()
    return grant


def mount_info_from_grant(grant: StorageGrant) -> StorageMountInfo:
    return StorageMountInfo(
        provider="renterd",
        endpoint=grant.endpoint,
        region=grant.region,
        bucket=grant.bucket,
        prefix=grant.prefix,
        path_style=grant.path_style,
        access_key_id=grant.access_key_id,
        secret_access_key=grant.secret_access_key,
        mount_inside_guest=grant.mount_inside_guest,
        rclone=grant.rclone,
    )
