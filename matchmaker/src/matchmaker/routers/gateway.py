"""Internal resolve API for the public gateway. Not a user-facing route."""

from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, status
from sqlalchemy.orm import Session

from matchmaker.db import get_db
from matchmaker.deps import require_gateway_resolver
from matchmaker.ingress import lookup_ingress_token
from matchmaker.schemas import GatewayResolveRequest, GatewayResolveResponse

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
    return GatewayResolveResponse(
        session_id=target.session_id,
        target_url=target.target_url,
        streamer=target.streamer,
    )
