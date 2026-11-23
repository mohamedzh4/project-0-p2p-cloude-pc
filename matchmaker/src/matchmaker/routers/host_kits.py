"""Setup kits a signed-in user downloads to share a PC (see host_kits.py)."""

from __future__ import annotations

from typing import Annotated, Literal

from fastapi import APIRouter, Depends, HTTPException, Response, status
from pydantic import Field
from sqlalchemy.orm import Session

from matchmaker.config import get_settings
from matchmaker.db import get_db
from matchmaker.deps import get_current_user
from matchmaker.host_kits import KIT_OSES, KitUnavailable, build_kit, kit_env, mint_code
from matchmaker.models import User
from matchmaker.schemas import CamelModel

router = APIRouter()


class HostKitRequest(CamelModel):
    os: Literal["windows", "linux"]


class HostKitOffer(CamelModel):
    os: str
    available: bool


class HostKitOffers(CamelModel):
    kits: list[HostKitOffer] = Field(default_factory=list)


@router.get("/host-kits", response_model=HostKitOffers)
def list_host_kits(_user: Annotated[User, Depends(get_current_user)]) -> HostKitOffers:
    """Which kits this server can make. A kit needs its download links set."""
    settings = get_settings()
    offers = []
    for lab_os in KIT_OSES:
        try:
            kit_env(settings, lab_os=lab_os, code="dclk_" + "x" * 40)
            available = True
        except KitUnavailable:
            available = False
        offers.append(HostKitOffer(os=lab_os, available=available))
    return HostKitOffers(kits=offers)


@router.post("/host-kits", responses={200: {"content": {"application/zip": {}}}})
def download_host_kit(
    payload: HostKitRequest,
    user: Annotated[User, Depends(get_current_user)],
    db: Annotated[Session, Depends(get_db)],
) -> Response:
    """A zip with the setup script and a registration code made for it."""
    settings = get_settings()
    try:
        kit_env(settings, lab_os=payload.os, code="dclk_" + "x" * 40)
    except KitUnavailable as exc:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="Host setup downloads are not available on this server yet",
        ) from exc
    code = mint_code(db, user_id=user.id, lab_os=payload.os)
    try:
        kit = build_kit(settings, lab_os=payload.os, code=code)
    except KitUnavailable as exc:
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail="Host setup downloads are not available on this server yet",
        ) from exc
    return Response(
        content=kit.body,
        media_type="application/zip",
        headers={
            "Content-Disposition": f'attachment; filename="{kit.filename}"',
            "Cache-Control": "no-store",
        },
    )
