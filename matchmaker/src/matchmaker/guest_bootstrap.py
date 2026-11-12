"""Guest-only bootstrap tokens (Headscale preauth + rclone locators).

The volunteer Host Agent must not receive a Headscale preauth or S3 keys.
Matchmaker mints an unguessable bootstrap token, puts origin+token on the
provision `guestMesh` hint (and then kernel cmdline / QEMU fw_cfg), and the
guest redeems `POST /v1/guest/bootstrap` for secrets.
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
from uuid import UUID
import hashlib
import secrets

from sqlalchemy import select
from sqlalchemy.orm import Session

from matchmaker.ingress import TOKEN_RE
from matchmaker.mesh import SessionMeshGrant, guest_mesh_hint, guest_tailscale_up_args
from matchmaker.models import LabSession, SessionGuestBootstrap, SessionStatus
from matchmaker.schemas import GuestBootstrapResponse, GuestBootstrapStorage
from matchmaker.storage import GuestStorageLocators, guest_rclone_args

_TOKEN_BYTES = 32


def hash_bootstrap_token(token: str) -> str:
    return hashlib.sha256(token.encode("utf-8")).hexdigest()


def new_bootstrap_token() -> str:
    token = secrets.token_urlsafe(_TOKEN_BYTES)
    if TOKEN_RE.fullmatch(token) is None:
        raise RuntimeError("generated guest bootstrap token failed format check")
    return token


def ensure_guest_bootstrap(
    db: Session,
    *,
    session_id: UUID,
    grant: SessionMeshGrant,
    storage: GuestStorageLocators,
    ttl_seconds: int,
    public_origin: str | None,
    now: datetime | None = None,
):
    """Store guest secrets keyed by a new token. Return the provision hint."""
    moment = _as_utc(now)
    existing = db.scalars(
        select(SessionGuestBootstrap).where(SessionGuestBootstrap.session_id == session_id)
    ).first()
    if existing is not None:
        db.delete(existing)
        db.flush()
    token = new_bootstrap_token()
    row = SessionGuestBootstrap(
        session_id=session_id,
        token_hash=hash_bootstrap_token(token),
        login_server=grant.login_server,
        hostname=grant.guest.hostname,
        exit_node=grant.exit_node,
        acl_tags=",".join(grant.guest.acl_tags),
        guest_preauth_key=grant.guest.preauth_key,
        storage_endpoint=storage.endpoint,
        storage_bucket=storage.bucket,
        storage_prefix=storage.prefix,
        storage_region=storage.region,
        expires_at=moment + timedelta(seconds=ttl_seconds),
    )
    db.add(row)
    db.commit()
    return guest_mesh_hint(
        grant,
        bootstrap_origin=public_origin,
        bootstrap_token=token,
    )


def lookup_guest_bootstrap(
    db: Session,
    token: str,
    *,
    now: datetime | None = None,
) -> SessionGuestBootstrap | None:
    if TOKEN_RE.fullmatch(token) is None:
        return None
    moment = _as_utc(now)
    digest = hash_bootstrap_token(token)
    row = db.scalars(
        select(SessionGuestBootstrap).where(SessionGuestBootstrap.token_hash == digest)
    ).first()
    if row is None:
        return None
    if row.expires_at <= moment:
        return None
    session = db.get(LabSession, row.session_id)
    if session is None or session.status != SessionStatus.ACTIVE:
        return None
    if row.redeemed_at is None:
        row.redeemed_at = moment
        db.commit()
        db.refresh(row)
    return row


def expire_session_bootstrap(db: Session, *, session_id: UUID) -> int:
    rows = db.scalars(
        select(SessionGuestBootstrap).where(SessionGuestBootstrap.session_id == session_id)
    ).all()
    count = len(rows)
    for row in rows:
        db.delete(row)
    if count:
        db.commit()
    return count


def record_saved_as_of(
    db: Session,
    row: SessionGuestBootstrap,
    *,
    saved_as_of: datetime | None = None,
) -> datetime:
    moment = _as_utc(saved_as_of)
    session = db.get(LabSession, row.session_id)
    if session is None:
        raise RuntimeError("bootstrap session is missing")
    session.saved_as_of = moment
    db.commit()
    db.refresh(session)
    assert session.saved_as_of is not None
    return session.saved_as_of


def bootstrap_response(
    row: SessionGuestBootstrap,
    *,
    access_key: str,
    secret_key: str,
) -> GuestBootstrapResponse:
    tags = tuple(tag for tag in row.acl_tags.split(",") if tag)
    rclone = guest_rclone_args(
        bucket=row.storage_bucket,
        prefix=row.storage_prefix,
        endpoint=row.storage_endpoint,
        region=row.storage_region,
    )
    return GuestBootstrapResponse(
        login_server=row.login_server,
        preauth_key=row.guest_preauth_key,
        hostname=row.hostname,
        exit_node=row.exit_node,
        acl_tags=list(tags),
        tailscale_up=list(
            guest_tailscale_up_args(
                login_server=row.login_server,
                auth_key=row.guest_preauth_key,
                hostname=row.hostname,
                exit_node=row.exit_node,
            )
        ),
        storage=GuestBootstrapStorage(
            endpoint=row.storage_endpoint,
            region=row.storage_region,
            bucket=row.storage_bucket,
            prefix=row.storage_prefix,
            rclone_args=rclone,
            env_auth=True,
            access_key_id=access_key,
            secret_access_key=secret_key,
        ),
    )


def _as_utc(now: datetime | None) -> datetime:
    moment = now if now is not None else datetime.now(timezone.utc)
    if moment.tzinfo is None:
        return moment.replace(tzinfo=timezone.utc)
    return moment.astimezone(timezone.utc)
