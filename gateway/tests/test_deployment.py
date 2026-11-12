from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def test_caddy_routes_app_api_and_desktops_to_distinct_origins() -> None:
    caddy = (ROOT / "gateway" / "Caddyfile").read_text(encoding="utf-8")
    assert "handle_path /api/*" in caddy
    assert "reverse_proxy 127.0.0.1:8000" in caddy
    assert "{$GATEWAY_DESKTOP_DOMAIN}" in caddy
    assert "reverse_proxy 127.0.0.1:7843" in caddy
    assert "{$GATEWAY_STORAGE_DOMAIN}" in caddy
    assert "reverse_proxy 127.0.0.1:8081" in caddy
    assert "{$GATEWAY_MESH_DOMAIN}" in caddy
    assert "reverse_proxy 127.0.0.1:8080" in caddy
    assert "reverse_proxy 127.0.0.1:7844" in caddy


def test_web_container_is_loopback_only_and_uses_same_origin_api() -> None:
    compose = (ROOT / "gateway" / "docker-compose.yml").read_text(encoding="utf-8")
    nginx = (ROOT / "web" / "nginx.conf").read_text(encoding="utf-8")
    assert "VITE_MATCHMAKER_URL: /api" in compose
    assert "network_mode: host" in compose
    assert "listen 127.0.0.1:7844;" in nginx
    assert "try_files $uri $uri/ /index.html;" in nginx
