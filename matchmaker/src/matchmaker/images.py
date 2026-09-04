"""Platform image registry metadata (digest + Ed25519 signature).

The Matchmaker advertises what the Host Agent must verify before a stub boot.
It does not store private keys, guest disks, or a CDN. Resource class is the
host's lab tier (`standard` / `pro`).
"""

from __future__ import annotations

from dataclasses import dataclass

from matchmaker.config import Settings
from matchmaker.models import LabTier


@dataclass(frozen=True)
class ImageManifest:
    lab_tier: str
    digest_sha256: str
    signature: str
    algorithm: str = "ed25519"
    digest_algorithm: str = "sha256"


def registry_from_settings(settings: Settings) -> dict[str, ImageManifest]:
    return {
        LabTier.STANDARD: ImageManifest(
            lab_tier=LabTier.STANDARD,
            digest_sha256=settings.image_standard_digest,
            signature=settings.image_standard_signature,
        ),
        LabTier.PRO: ImageManifest(
            lab_tier=LabTier.PRO,
            digest_sha256=settings.image_pro_digest,
            signature=settings.image_pro_signature,
        ),
    }


def manifest_for(settings: Settings, lab_tier: str) -> ImageManifest | None:
    return registry_from_settings(settings).get(lab_tier)
