from __future__ import annotations

import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import Thread
from uuid import uuid4

from fastapi.testclient import TestClient

from matchmaker.images import ImageManifest
from matchmaker.models import Host, HostStatus, LabTier
from matchmaker.provision import (
    STREAMER_SUNSHINE,
    HttpHostProvisioner,
    agent_origin,
    session_connection,
)
from tests.helpers import (
    KASM_TOKEN,
    KASM_URL,
    PROVISION_URL,
    SUNSHINE_URL,
    inventory_payload,
    host_auth_header,
)
from tests.image_fixtures import STANDARD_DIGEST, STANDARD_SIGNATURE


def _host(*, agent_url: str | None, shared_gpu_id: str | None = None) -> Host:
    return Host(
        id=uuid4(),
        display_name="agent-host",
        lab_tier=LabTier.STANDARD,
        shared_logical_cores=4,
        shared_memory_bytes=8 * 1024 * 1024 * 1024,
        share_gpu=shared_gpu_id is not None,
        shared_gpu_id=shared_gpu_id,
        kasm_url="https://kasm.test.example/session",
        kasm_token="test-kasm-token",
        agent_url=agent_url,
        status=HostStatus.APPROVED,
    )


def test_agent_origin_rejects_non_http_and_embedded_credentials() -> None:
    assert agent_origin(None) is None
    assert agent_origin("  ") is None
    assert agent_origin("ftp://127.0.0.1:9") is None
    assert agent_origin("http://user:pass@127.0.0.1:9") is None
    assert agent_origin("http://127.0.0.1:7842/") == "http://127.0.0.1:7842"


def test_http_provisioner_returns_none_without_an_agent_url() -> None:
    provisioner = HttpHostProvisioner(bearer_token="test-host-bootstrap-token")
    assert provisioner.provision(_host(agent_url=None), lab_tier="standard") is None


def test_http_provisioner_posts_to_the_agent_and_reads_connection_url() -> None:
    captured: dict[str, object] = {}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:  # noqa: A003
            return

        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length", "0"))
            captured["path"] = self.path
            captured["authorization"] = self.headers.get("Authorization")
            captured["body"] = json.loads(self.rfile.read(length))
            payload = json.dumps(
                {
                    "connectionUrl": PROVISION_URL,
                    "hypervisorInvoked": False,
                    "connectionToken": "stub-token",
                }
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        origin = f"http://127.0.0.1:{server.server_address[1]}"
        provisioner = HttpHostProvisioner(bearer_token="test-host-bootstrap-token")
        result = provisioner.provision(_host(agent_url=origin), lab_tier="standard")
    finally:
        server.shutdown()
        thread.join(timeout=2)

    assert result is not None
    assert result.connection_url == PROVISION_URL
    assert result.connection_token == "stub-token"
    assert captured["path"] == "/v1/provision"
    assert captured["authorization"] == "Bearer test-host-bootstrap-token"
    assert captured["body"] == {"labTier": "standard", "sharedGpuId": None}


def test_http_provisioner_posts_storage_endpoint_without_s3_secrets() -> None:
    captured: dict[str, object] = {}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:  # noqa: A003
            return

        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length", "0"))
            captured["body"] = json.loads(self.rfile.read(length))
            payload = json.dumps(
                {"connectionUrl": PROVISION_URL, "hypervisorInvoked": False}
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        origin = f"http://127.0.0.1:{server.server_address[1]}"
        provisioner = HttpHostProvisioner(
            bearer_token="test-host-bootstrap-token",
            storage_endpoint="http://renterd.test.invalid",
        )
        result = provisioner.provision(_host(agent_url=origin), lab_tier="standard")
    finally:
        server.shutdown()
        thread.join(timeout=2)

    assert result is not None
    body = captured["body"]
    assert isinstance(body, dict)
    assert body["storageEndpoint"] == "http://renterd.test.invalid"
    assert "secretAccessKey" not in body
    assert "accessKeyId" not in body
    assert "RENTERD_S3_SECRET_KEY" not in json.dumps(body)


def test_http_provisioner_advertises_signed_image_metadata() -> None:
    captured: dict[str, object] = {}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:  # noqa: A003
            return

        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length", "0"))
            captured["body"] = json.loads(self.rfile.read(length))
            payload = json.dumps(
                {"connectionUrl": PROVISION_URL, "hypervisorInvoked": False}
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        origin = f"http://127.0.0.1:{server.server_address[1]}"
        provisioner = HttpHostProvisioner(
            bearer_token="test-host-bootstrap-token",
            images={
                "standard": ImageManifest(
                    lab_tier="standard",
                    digest_sha256=STANDARD_DIGEST,
                    signature=STANDARD_SIGNATURE,
                )
            },
        )
        result = provisioner.provision(_host(agent_url=origin), lab_tier="standard")
    finally:
        server.shutdown()
        thread.join(timeout=2)

    assert result is not None
    assert captured["body"] == {
        "labTier": "standard",
        "sharedGpuId": None,
        "imageDigest": STANDARD_DIGEST,
        "imageSignature": STANDARD_SIGNATURE,
    }


def test_http_provisioner_uses_fallback_origin_when_host_has_none() -> None:
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:  # noqa: A003
            return

        def do_POST(self) -> None:  # noqa: N802
            payload = json.dumps(
                {"connectionUrl": PROVISION_URL, "hypervisorInvoked": False}
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        origin = f"http://127.0.0.1:{server.server_address[1]}"
        provisioner = HttpHostProvisioner(
            fallback_base_url=origin,
            bearer_token="test-host-bootstrap-token",
        )
        result = provisioner.provision(_host(agent_url=None), lab_tier="standard")
    finally:
        server.shutdown()
        thread.join(timeout=2)

    assert result is not None
    assert result.connection_url == PROVISION_URL


def test_http_provisioner_treats_agent_errors_as_unavailable() -> None:
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:  # noqa: A003
            return

        def do_POST(self) -> None:  # noqa: N802
            self.send_response(401)
            self.end_headers()

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        origin = f"http://127.0.0.1:{server.server_address[1]}"
        provisioner = HttpHostProvisioner(bearer_token="wrong-token")
        result = provisioner.provision(_host(agent_url=origin), lab_tier="standard")
    finally:
        server.shutdown()
        thread.join(timeout=2)

    assert result is None


def test_http_provisioner_refuses_a_response_that_claims_a_hypervisor() -> None:
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:  # noqa: A003
            return

        def do_POST(self) -> None:  # noqa: N802
            payload = json.dumps(
                {"connectionUrl": PROVISION_URL, "hypervisorInvoked": True}
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        origin = f"http://127.0.0.1:{server.server_address[1]}"
        provisioner = HttpHostProvisioner(bearer_token="test-host-bootstrap-token")
        result = provisioner.provision(_host(agent_url=origin), lab_tier="standard")
    finally:
        server.shutdown()
        thread.join(timeout=2)

    assert result is None


def test_http_provisioner_reads_pro_sunshine_fields() -> None:
    captured: dict[str, object] = {}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *args: object) -> None:  # noqa: A003
            return

        def do_POST(self) -> None:  # noqa: N802
            length = int(self.headers.get("Content-Length", "0"))
            captured["body"] = json.loads(self.rfile.read(length))
            payload = json.dumps(
                {
                    "connectionUrl": "https://provision.invalid/qemu-vfio",
                    "hypervisorInvoked": False,
                    "gpuExclusive": True,
                    "streamer": "sunshine",
                    "sunshineUrl": SUNSHINE_URL,
                    "moonlight": {"host": "sunshine.provision.invalid", "app": "Desktop"},
                }
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        origin = f"http://127.0.0.1:{server.server_address[1]}"
        provisioner = HttpHostProvisioner(bearer_token="test-host-bootstrap-token")
        result = provisioner.provision(
            _host(agent_url=origin, shared_gpu_id="GPU-TEST-4070"),
            lab_tier="pro",
        )
    finally:
        server.shutdown()
        thread.join(timeout=2)

    assert result is not None
    assert result.streamer == STREAMER_SUNSHINE
    assert result.sunshine_url == SUNSHINE_URL
    assert result.locator() == SUNSHINE_URL
    assert result.gpu_exclusive is True
    assert result.moonlight_host == "sunshine.provision.invalid"
    body = captured["body"]
    assert isinstance(body, dict)
    assert body["labTier"] == "pro"
    assert body["sharedGpuId"] == "GPU-TEST-4070"


def test_session_connection_is_kasm_for_standard() -> None:
    host = _host(agent_url=None)
    info = session_connection(host)
    dumped = info.model_dump(by_alias=True)
    assert dumped["streamer"] == "kasm"
    assert dumped["kasmUrl"] == KASM_URL
    assert dumped["kasmToken"] == "test-kasm-token"
    assert dumped["sunshineUrl"] is None
    assert dumped["moonlight"] is None
    assert dumped["gpuExclusive"] is False


def test_session_connection_is_sunshine_for_pro() -> None:
    host = _host(agent_url=None, shared_gpu_id="GPU-0")
    host.lab_tier = LabTier.PRO
    host.kasm_url = SUNSHINE_URL
    host.kasm_token = KASM_TOKEN
    info = session_connection(host)
    dumped = info.model_dump(by_alias=True)
    assert dumped["streamer"] == "sunshine"
    assert dumped["kasmUrl"] is None
    assert dumped["kasmToken"] is None
    assert dumped["sunshineUrl"] == SUNSHINE_URL
    assert dumped["moonlight"]["host"] == "provision.invalid"
    assert dumped["gpuExclusive"] is True


def test_agent_url_round_trips_on_register(client: TestClient) -> None:
    response = client.post(
        "/v1/hosts",
        headers=host_auth_header(),
        json=inventory_payload(agentUrl="http://127.0.0.1:7842"),
    )
    assert response.status_code == 201
    assert response.json()["agentUrl"] == "http://127.0.0.1:7842"
    assert response.json()["status"] == "pending"


def test_put_without_agent_url_does_not_wipe_it(client: TestClient) -> None:
    created = client.post(
        "/v1/hosts",
        headers=host_auth_header(),
        json=inventory_payload(agentUrl="http://127.0.0.1:7842"),
    )
    host_id = created.json()["hostId"]
    updated = client.put(
        f"/v1/hosts/{host_id}",
        headers=host_auth_header(),
        json=inventory_payload(sharedLogicalCores=8),
    )
    assert updated.status_code == 200
    assert updated.json()["agentUrl"] == "http://127.0.0.1:7842"
    assert updated.json()["sharedLogicalCores"] == 8


def test_invalid_agent_url_is_rejected(client: TestClient) -> None:
    response = client.post(
        "/v1/hosts",
        headers=host_auth_header(),
        json=inventory_payload(agentUrl="not-a-url"),
    )
    assert response.status_code == 422
