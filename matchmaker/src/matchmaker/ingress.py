"""Scoped public-gateway tokens (architecture §9 item 9).

A browser on the public internet cannot reach a Headscale mesh locator without
joining the tailnet. Matchmaker mints an unguessable token that maps to one
session's Kasm/Sunshine URL. The gateway looks that mapping up; it never
falls through to a default session.

Headscale 0.26 file policy still cannot mint pairwise `tag:session-<uuid>`
owners, so `tag:gateway` may reach every `tag:guest` the same way `tag:user`
can. Session isolation on the public URL is this token, not the ACL file.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from uuid import UUID
import hashlib
import re
import secrets

from sqlalchemy import delete, or_, select
from sqlalchemy.orm import Session

from matchmaker.models import LabSession, SessionDesktopLink, SessionIngressToken, SessionStatus
from matchmaker.provision import STREAMER_KASM, STREAMER_SUNSHINE, locator_is_placeholder
from matchmaker.schemas import ConnectionInfo

TOKEN_RE = re.compile(r"^[A-Za-z0-9_-]{32,64}$")
_TOKEN_BYTES = 32
# A desktop link works once, within this time. The web app asks for the
# session every 10 s while it is active, so the link it shows stays fresh.
DESKTOP_LINK_TTL_SECONDS = 120


@dataclass(frozen=True)
class IngressTarget:
    """What the gateway may proxy to. Never a default or sibling session."""

    session_id: UUID
    target_url: str
    streamer: str


def hash_ingress_token(token: str) -> str:
    return hashlib.sha256(token.encode("utf-8")).hexdigest()


def new_ingress_token() -> str:
    token = secrets.token_urlsafe(_TOKEN_BYTES)
    if TOKEN_RE.fullmatch(token) is None:
        raise RuntimeError("generated ingress token failed format check")
    return token


def public_locator_url(origin: str, token: str) -> str:
    return f"{origin.rstrip('/')}/s/{token}/"


def ensure_session_ingress(
    db: Session,
    *,
    session_id: UUID,
    target_url: str,
    streamer: str,
    ttl_seconds: int,
    now: datetime | None = None,
) -> SessionIngressToken:
    """Reuse the session's token (same public URL) and refresh the mesh target."""
    moment = _as_utc(now)
    row = db.scalars(
        select(SessionIngressToken).where(SessionIngressToken.session_id == session_id)
    ).first()
    if row is not None and row.expires_at > moment:
        row.target_url = target_url
        row.streamer = streamer
        db.commit()
        db.refresh(row)
        return row
    if row is not None:
        db.delete(row)
        db.flush()
    token = new_ingress_token()
    created = SessionIngressToken(
        session_id=session_id,
        token_hash=hash_ingress_token(token),
        token=token,
        target_url=target_url,
        streamer=streamer,
        expires_at=moment + timedelta(seconds=ttl_seconds),
    )
    db.add(created)
    db.commit()
    db.refresh(created)
    return created


def lookup_ingress_token(
    db: Session,
    token: str,
    *,
    now: datetime | None = None,
) -> IngressTarget | None:
    """Fail closed: wrong, expired, or inactive tokens map to nothing."""
    if TOKEN_RE.fullmatch(token) is None:
        return None
    moment = _as_utc(now)
    digest = hash_ingress_token(token)
    row = db.scalars(
        select(SessionIngressToken).where(SessionIngressToken.token_hash == digest)
    ).first()
    if row is None:
        return None
    if row.expires_at <= moment:
        return None
    session = db.get(LabSession, row.session_id)
    if session is None or session.status != SessionStatus.ACTIVE:
        return None
    if locator_is_placeholder(row.target_url):
        return None
    if row.redeemed_at is None:
        row.redeemed_at = moment
        db.commit()
        db.refresh(row)
    return IngressTarget(
        session_id=row.session_id,
        target_url=row.target_url,
        streamer=row.streamer,
    )


def expire_session_ingress(db: Session, *, session_id: UUID) -> int:
    rows = db.scalars(
        select(SessionIngressToken).where(SessionIngressToken.session_id == session_id)
    ).all()
    count = len(rows)
    for row in rows:
        db.delete(row)
    links = db.execute(
        delete(SessionDesktopLink).where(SessionDesktopLink.session_id == session_id)
    ).rowcount
    if count or links:
        db.commit()
    return count


def mint_desktop_link(
    db: Session,
    *,
    session_id: UUID,
    ttl_seconds: int = DESKTOP_LINK_TTL_SECONDS,
    now: datetime | None = None,
) -> str:
    """A new single-use link token for this session. Drops its dead links."""
    moment = _as_utc(now)
    db.execute(
        delete(SessionDesktopLink).where(
            SessionDesktopLink.session_id == session_id,
            or_(
                SessionDesktopLink.expires_at <= moment,
                SessionDesktopLink.used_at.is_not(None),
            ),
        )
    )
    token = new_ingress_token()
    db.add(
        SessionDesktopLink(
            session_id=session_id,
            token_hash=hash_ingress_token(token),
            expires_at=moment + timedelta(seconds=ttl_seconds),
        )
    )
    db.commit()
    return token


@dataclass(frozen=True)
class RedeemedLink:
    target: IngressTarget
    # The session's ingress token, for the gateway to set as an HttpOnly cookie.
    cookie_token: str


def redeem_desktop_link(
    db: Session,
    link_token: str,
    *,
    browser_cookie: str | None = None,
    now: datetime | None = None,
) -> RedeemedLink | None:
    """Trade a desktop link for the session's cookie token, once.

    A used link still works for the browser that used it (a second tab, a
    double click): that browser already holds this session's cookie. Anyone
    else gets nothing.
    """
    if TOKEN_RE.fullmatch(link_token) is None:
        return None
    moment = _as_utc(now)
    link = db.scalars(
        select(SessionDesktopLink)
        .where(SessionDesktopLink.token_hash == hash_ingress_token(link_token))
        .with_for_update()
    ).first()
    if link is None or link.expires_at <= moment:
        db.rollback()
        return None
    grant = db.scalars(
        select(SessionIngressToken).where(SessionIngressToken.session_id == link.session_id)
    ).first()
    if grant is None:
        db.rollback()
        return None
    if link.used_at is not None:
        same_browser = browser_cookie is not None and secrets.compare_digest(
            hash_ingress_token(browser_cookie), grant.token_hash
        )
        if not same_browser:
            db.rollback()
            return None
    else:
        link.used_at = moment
        db.commit()
    target = lookup_ingress_token(db, grant.token, now=moment)
    if target is None:
        return None
    return RedeemedLink(target=target, cookie_token=grant.token)


def connection_for_browser(
    db: Session,
    session: LabSession,
    inner: ConnectionInfo,
    *,
    public_origin: str | None,
    ttl_seconds: int,
) -> ConnectionInfo:
    """Replace the mesh locator with a gateway URL when the origin is configured.

    Moonlight host/app stay on the mesh hostname — Pro native clients are not
    the zero-install Standard Lab path. Provisioner logic is unchanged.
    """
    if not public_origin:
        return inner
    locator = _inner_locator(inner)
    if locator is None or locator_is_placeholder(locator):
        return inner
    grant = ensure_session_ingress(
        db,
        session_id=session.id,
        target_url=locator,
        streamer=inner.streamer,
        ttl_seconds=ttl_seconds,
    )
    # The URL carries a single-use link, never the session's own token.
    link = mint_desktop_link(db, session_id=grant.session_id)
    public = public_locator_url(public_origin, link)
    if inner.streamer == STREAMER_SUNSHINE:
        return inner.model_copy(update={"sunshine_url": public})
    if inner.streamer == STREAMER_KASM:
        return inner.model_copy(update={"kasm_url": public})
    return inner


def _inner_locator(inner: ConnectionInfo) -> str | None:
    if inner.streamer == STREAMER_SUNSHINE:
        return inner.sunshine_url
    return inner.kasm_url


def _as_utc(now: datetime | None) -> datetime:
    moment = now if now is not None else datetime.now(timezone.utc)
    if moment.tzinfo is None:
        return moment.replace(tzinfo=timezone.utc)
    return moment.astimezone(timezone.utc)
