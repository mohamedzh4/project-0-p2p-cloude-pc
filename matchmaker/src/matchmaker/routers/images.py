from __future__ import annotations

from fastapi import APIRouter, HTTPException, status

from matchmaker.config import get_settings
from matchmaker.images import manifest_for
from matchmaker.schemas import ImageRegistryResponse

router = APIRouter()


@router.get("/images/{lab_tier}", response_model=ImageRegistryResponse)
def get_signed_image(lab_tier: str) -> ImageRegistryResponse:
    """Public checksums for a resource class. Not a blob download."""
    manifest = manifest_for(get_settings(), lab_tier.strip().lower())
    if manifest is None:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND, detail="unknown lab tier")
    return ImageRegistryResponse(
        lab_tier=manifest.lab_tier,
        digest_sha256=manifest.digest_sha256,
        signature=manifest.signature,
        algorithm=manifest.algorithm,
        digest_algorithm=manifest.digest_algorithm,
    )
