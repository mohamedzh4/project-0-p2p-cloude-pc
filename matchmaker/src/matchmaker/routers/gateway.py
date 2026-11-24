"""Internal resolve API for the public gateway. Not a user-facing route."""

from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, status
from sqlalchemy.orm import Session

from matchmaker.db import get_db
from matchmaker.deps import require_gateway_resolver
from matchmaker.guest_bootstrap import desktop_credentials
from matchmaker.ingress import IngressTarget, lookup_ingress_token, redeem_desktop_link
from matchmaker.schemas import (
    GatewayRedeemRequest,
    GatewayRedeemResponse,
    GatewayResolveRequest,
    GatewayResolveResponse,
)

router = APIRouter()


@router.post("/gateway/resolve", response_model=GatewayResolveResponse)
def resolve_gateway_token(
    payload: GatewayResolveRequest,
    db: Annotated[Session, Depends(get_db)],
    _: Annotated[None, Depends(require_gateway_resolver)],
) -> GatewayResolveResponse:
    target = lookup_ingress_token(db, payload.token)
    if target is None:
        raise HTTPException(
            status_code=status.HTTP_404_NOT_FOUND,
            detail="Session not found",
        )
    credentials = desktop_credentials(db, target.session_id)
    return GatewayResolveResponse(
        session_id=target.session_id,
        target_url=target.target_url,
        streamer=target.streamer,
        upstream_user=credentials[0] if credentials else None,
        upstream_password=credentials[1] if credentials else None,
    )


def _not_found() -> HTTPException:
    return HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="Session not found")


def _resolved(db: Session, target: IngressTarget) -> dict:
    credentials = desktop_credentials(db, target.session_id)
    return {
        "session_id": target.session_id,
        "target_url": target.target_url,
        "streamer": target.streamer,
        "upstream_user": credentials[0] if credentials else None,
        "upstream_password": credentials[1] if credentials else None,
    }


@router.post("/gateway/redeem", response_model=GatewayRedeemResponse)
def redeem_gateway_link(
    payload: GatewayRedeemRequest,
    db: Annotated[Session, Depends(get_db)],
    _: Annotated[None, Depends(require_gateway_resolver)],
) -> GatewayRedeemResponse:
    """A browser opened a desktop link: give the gateway the session cookie."""
    redeemed = redeem_desktop_link(db, payload.token, browser_cookie=payload.cookie_token)
    if redeemed is None:
        raise _not_found()
    return GatewayRedeemResponse(
        **_resolved(db, redeemed.target), cookie_token=redeemed.cookie_token
    )
