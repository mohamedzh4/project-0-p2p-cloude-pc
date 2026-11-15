from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

from tests.image_fixtures import PRO_DIGEST, PRO_SIGNATURE, STANDARD_DIGEST, STANDARD_SIGNATURE


def test_image_registry_returns_standard_and_pro_metadata(client: TestClient) -> None:
    standard = client.get("/v1/images/standard")
    assert standard.status_code == 200
    body = standard.json()
    assert body == {
        "labTier": "standard",
        "digestSha256": STANDARD_DIGEST,
        "signature": STANDARD_SIGNATURE,
        "algorithm": "ed25519",
        "digestAlgorithm": "sha256",
    }

    pro = client.get("/v1/images/pro")
    assert pro.status_code == 200
    assert pro.json()["digestSha256"] == PRO_DIGEST
    assert pro.json()["signature"] == PRO_SIGNATURE
    assert pro.json()["algorithm"] == "ed25519"


def test_image_registry_unknown_tier_is_404(client: TestClient) -> None:
    response = client.get("/v1/images/gamer")
    assert response.status_code == 404
    assert response.json()["detail"] == "unknown lab tier"


def test_image_and_session_routers_do_not_name_backends_or_the_concrete_selector() -> None:
    root = Path(__file__).resolve().parents[1] / "src/matchmaker"
    for relative in ("routers/images.py", "images.py", "provision.py"):
        text = (root / relative).read_text(encoding="utf-8")
        assert "SingleApprovedHostSelector" not in text
        assert "RankedHostSelector" not in text
        assert "QEMU" not in text
        assert "WHPX" not in text
        assert "VFIO" not in text
