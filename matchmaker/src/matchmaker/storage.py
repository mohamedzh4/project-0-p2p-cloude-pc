"""Per-user renterd bucket records and guest rclone instructions.

Mount belongs **inside the guest**. Never rclone on the volunteer host OS.
S3 credentials stay in Matchmaker/guest env — never on session JSON.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from uuid import UUID

from sqlalchemy.orm import Session

from matchmaker.models import User, UserStorage
from matchmaker.renterd import RenterdClient, StorageGrant
from matchmaker.schemas import StorageMountInfo

USER_FILES_PREFIX = "files/"
PROVISION_MARKER_KEY = f"{USER_FILES_PREFIX}.dcl/provisioned"
DEFAULT_GUEST_MOUNT = "/home/ubuntu/Persistent"
GUEST_ENV_AUTH_VARS = ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY")


@dataclass(frozen=True)
class GuestStorageLocators:
    """Public S3 locators for a guest rclone plan. No access keys."""

    endpoint: str
    bucket: str
    prefix: str
    region: str
    mount_point: str = DEFAULT_GUEST_MOUNT


def bucket_name_for_user(user_id: UUID) -> str:
    """S3-legal per-user bucket. Hyphenated UUID, prefixed so it is not global."""
    return f"dcl-{user_id}"


def shared_prefix_for_user(user_id: UUID) -> str:
    """Per-user prefix inside one shared bucket. Credentials are limited to it."""
    return f"users/{user_id}/{USER_FILES_PREFIX}"


def location_for_user(user_id: UUID, *, shared_bucket: str | None) -> tuple[str, str]:
    """(bucket, prefix) for a user: shared bucket + own prefix, or own bucket."""
    if shared_bucket:
        return shared_bucket, shared_prefix_for_user(user_id)
    return bucket_name_for_user(user_id), USER_FILES_PREFIX


def _is_r2_endpoint(endpoint: str) -> bool:
    return endpoint.rstrip("/").lower().endswith(".r2.cloudflarestorage.com")


def guest_rclone_remote(*, bucket: str, prefix: str) -> str:
    remote = f":s3:{bucket}"
    trimmed = prefix.strip("/")
    if trimmed:
        remote = f"{remote}/{trimmed}"
    return remote


def guest_rclone_args(
    *,
    bucket: str,
    prefix: str,
    endpoint: str,
    region: str = "us-east-1",
    mount_point: str = DEFAULT_GUEST_MOUNT,
    read_only: bool = False,
) -> list[str]:
    """rclone argv for the guest. Credentials stay in env (`--s3-env-auth`), not argv.

    `--s3-no-check-bucket`: a per-user credential may not inspect or create
    the bucket itself, only objects under its prefix.
    """
    args = [
        "mount",
        guest_rclone_remote(bucket=bucket, prefix=prefix),
        mount_point,
        "--s3-provider",
        "Other",
        "--s3-endpoint",
        endpoint,
        "--s3-env-auth",
        "--s3-force-path-style",
        "--s3-region",
        region,
        "--s3-no-check-bucket",
        "--vfs-cache-mode",
        "full",
    ]
    if not _is_r2_endpoint(endpoint):
        # renterd (Sia) wants large parts. On R2 this would hold up to
        # 120 MiB x 4 in the guest's memory for every upload, so rclone's
        # small default is kept there.
        args += ["--s3-chunk-size", "120MiB", "--fast-list"]
    if read_only:
        args.append("--read-only")
    return args


def guest_rclone_mount_command(
    *,
    bucket: str,
    prefix: str,
    endpoint: str,
    region: str = "us-east-1",
    mount_point: str = DEFAULT_GUEST_MOUNT,
) -> str:
    """Shell-shaped display string. Prefer `guest_rclone_args` for exec."""
    return "rclone " + " ".join(
        guest_rclone_args(
            bucket=bucket,
            prefix=prefix,
            endpoint=endpoint,
            region=region,
            mount_point=mount_point,
        )
    )


def locators_from_grant(grant: StorageGrant) -> GuestStorageLocators:
    return GuestStorageLocators(
        endpoint=grant.endpoint,
        bucket=grant.bucket,
        prefix=grant.prefix,
        region=grant.region,
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
            bucket=resolved_bucket,
            prefix=prefix,
            endpoint=endpoint,
            region=region,
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
    shared_bucket: str | None = None,
) -> StorageGrant:
    """Record where this user's files live, creating the place if needed. Idempotent.

    With `shared_bucket` the bucket must already exist; only the user's
    marker object is written. Otherwise a bucket per user is created.
    """
    bucket, prefix = location_for_user(user.id, shared_bucket=shared_bucket)
    grant = storage_grant(
        user_id=user.id,
        endpoint=endpoint,
        region=region,
        bucket=bucket,
        prefix=prefix,
    )
    marker_key = f"{prefix}.dcl/provisioned"
    if shared_bucket:
        created = db.get(UserStorage, user.id) is None
    else:
        created = renterd.create_bucket(grant.bucket)
    if created:
        marker = f"provisioned {datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')}\n"
        renterd.put_object(grant.bucket, marker_key, marker.encode("utf-8"))

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


def mount_info_from_grant(
    grant: StorageGrant,
    *,
    saved_as_of: datetime | None = None,
) -> StorageMountInfo:
    args = guest_rclone_args(
        bucket=grant.bucket,
        prefix=grant.prefix,
        endpoint=grant.endpoint,
        region=grant.region,
    )
    return StorageMountInfo(
        provider="renterd",
        endpoint=grant.endpoint,
        region=grant.region,
        bucket=grant.bucket,
        prefix=grant.prefix,
        path_style=grant.path_style,
        mount_inside_guest=grant.mount_inside_guest,
        rclone=grant.rclone,
        rclone_args=args,
        env_auth=True,
        objects=list(grant.objects),
        saved_as_of=saved_as_of,
    )
