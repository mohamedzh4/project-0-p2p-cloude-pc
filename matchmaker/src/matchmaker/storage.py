"""Per-user renterd bucket records and guest rclone instructions.

Mount belongs **inside the guest**. Never rclone on the volunteer host OS.
S3 credentials stay in Matchmaker/guest env — never on session JSON.
"""

from __future__ import annotations

from datetime import datetime, timezone
from uuid import UUID

from sqlalchemy.orm import Session

from matchmaker.models import User, UserStorage
from matchmaker.renterd import RenterdClient, StorageGrant
from matchmaker.schemas import StorageMountInfo

USER_FILES_PREFIX = "files/"
PROVISION_MARKER_KEY = f"{USER_FILES_PREFIX}.dcl/provisioned"


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
    prefix: str = USER_FILES_PREFIX,
    objects: tuple[str, ...] = (),
    bucket: str | None = None,
) -> StorageGrant:
    resolved_bucket = bucket if bucket is not None else bucket_name_for_user(user_id)
    return StorageGrant(
        endpoint=endpoint,
        region=region,
        bucket=resolved_bucket,
        prefix=prefix,
        path_style=True,
        mount_inside_guest=True,
        rclone=guest_rclone_mount_command(
            bucket=resolved_bucket, prefix=prefix, endpoint=endpoint
        ),
        objects=objects,
    )


def grant_from_row(
    row: UserStorage,
    renterd: RenterdClient,
    *,
    endpoint: str,
    region: str,
) -> StorageGrant:
    objects = tuple(renterd.list_object_keys(row.bucket, row.prefix))
    return storage_grant(
        user_id=row.user_id,
        endpoint=endpoint,
        region=region,
        prefix=row.prefix,
        objects=objects,
        bucket=row.bucket,
    )


def ensure_user_storage(
    db: Session,
    user: User,
    renterd: RenterdClient,
    *,
    endpoint: str,
    region: str,
) -> StorageGrant:
    """Create the per-user bucket if needed and record it. Idempotent."""
    grant = storage_grant(
        user_id=user.id,
        endpoint=endpoint,
        region=region,
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
    objects = tuple(renterd.list_object_keys(grant.bucket, grant.prefix))
    return storage_grant(
        user_id=user.id,
        endpoint=endpoint,
        region=region,
        objects=objects,
        bucket=grant.bucket,
        prefix=grant.prefix,
    )


def mount_info_from_grant(grant: StorageGrant) -> StorageMountInfo:
    return StorageMountInfo(
        provider="renterd",
        endpoint=grant.endpoint,
        region=grant.region,
        bucket=grant.bucket,
        prefix=grant.prefix,
        path_style=grant.path_style,
        mount_inside_guest=grant.mount_inside_guest,
        rclone=grant.rclone,
        objects=list(grant.objects),
    )
