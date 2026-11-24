#!/usr/bin/env python3
"""Integration test: the real Matchmaker and gateway, over real HTTP.

Unit tests fake every boundary inside one process. This starts the
Matchmaker and the public gateway as separate processes against a real
Postgres database and drives a whole booking through them:

    register host -> approve -> heartbeat -> sign up -> book -> poll until
    ready -> open the desktop through the gateway -> end -> check credits

What is real: Matchmaker, gateway, Postgres, the migrations, the background
provision thread, the stale-session sweep, every HTTP hop between them.

What is a stand-in (in this file): the Host Agent, the guest VM, Headscale,
the S3 store, the Cloudflare credential API and the desktop. The stand-in
agent speaks the same HTTP contract as the Rust agent (command-token
Bearer, `/v1/provision`, `/v1/teardown`) and plays the guest too (it redeems
the guest bootstrap). No VM is started. Real hardware is Phase 5.

Run it (Postgres must be reachable):

    bash scripts/check.sh integration
    # or: matchmaker/.venv/bin/python scripts/integration/run.py

It uses its own database, `matchmaker_integration`, on the server named by
INTEGRATION_DATABASE_URL, else TEST_DATABASE_URL. That database is dropped
and recreated on every run.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import secrets
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DB_NAME = "matchmaker_integration"

# Local-dev image vectors (same as matchmaker/tests/image_fixtures.py).
STANDARD_DIGEST = "6ed707af13eb04a928001996ae5fc7a49ab405e73534caacf86ad65c51bab8fe"
STANDARD_SIGNATURE = (
    "WQjS2zyW+BjgfoZDK3IFx6OIwXPmRxO22PN4FpfEJNpjY/AgEsLmqLVak99QZyY14YKNI5hiARiPkulHNpsrCQ=="
)
PRO_DIGEST = "b5dc04d095774efc8fee56dfcbabbe7346788f29a7576aaa27248d9b8249b118"
PRO_SIGNATURE = (
    "T1FrV8mlhTvhk+0laH74qnb6s6CTM2M+waEhIAtjszKlN3pZyQ+w18xDfp0xvbA3MO+FXvt8Z+B+hmXDrvBaDA=="
)

BOOTSTRAP_TOKEN = "integration-host-bootstrap-token"
APPROVE_TOKEN = "integration-host-approve-token"
RESOLVE_TOKEN = "integration-gateway-resolve-token"
S3_ACCESS_KEY = "integration-master-access-key"
S3_SECRET_KEY = "integration-master-secret-key"
R2_API_TOKEN = "integration-cloudflare-api-token"
SHARED_BUCKET = "dcl-files"
STALE_SECONDS = 4
HOST_SECRET_HEADER = "X-DCL-Host-Secret"


# --------------------------------------------------------------------------
# small helpers


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def command_token(secret: str) -> str:
    """Must match host_credentials.py and host_secret.rs."""
    return hashlib.sha256(("dcl-agent-command:" + secret).encode()).hexdigest()


@dataclass
class Reply:
    status: int
    body: bytes
    headers: dict[str, str]

    def json(self):
        return json.loads(self.body or b"null")

    @property
    def text(self) -> str:
        return self.body.decode("utf-8", "replace")


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        return None


_opener = urllib.request.build_opener(_NoRedirect)


def http(method: str, url: str, *, body=None, headers=None, timeout: float = 15) -> Reply:
    data = None
    send_headers = dict(headers or {})
    if body is not None:
        data = json.dumps(body).encode()
        send_headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, method=method, headers=send_headers)
    try:
        with _opener.open(request, timeout=timeout) as response:
            return Reply(response.status, response.read(), dict(response.headers))
    except urllib.error.HTTPError as error:
        return Reply(error.code, error.read(), dict(error.headers))


class Checks:
    def __init__(self) -> None:
        self.passed = 0
        self.failed: list[str] = []

    def section(self, title: str) -> None:
        print(f"\n== {title}")

    def check(self, label: str, condition: bool, detail: object = "") -> bool:
        if condition:
            self.passed += 1
            print(f"  OK   {label}")
        else:
            self.failed.append(label)
            print(f"  FAIL {label}" + (f"\n       {detail}" if detail != "" else ""))
        return bool(condition)


# --------------------------------------------------------------------------
# stand-in services


def serve(handler_factory) -> tuple[ThreadingHTTPServer, str]:
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler_factory)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_address[1]}"


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args) -> None:  # keep the output readable
        pass

    def read_json(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            return json.loads(raw) if raw else {}
        except ValueError:
            return {}

    def reply(self, status: int, body, content_type: str = "application/json") -> None:
        payload = body if isinstance(body, bytes) else json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(payload)


@dataclass
class FakeHeadscale:
    minted: list[dict] = field(default_factory=list)
    nodes: dict[str, str] = field(default_factory=dict)  # node id -> hostname
    expired: list[str] = field(default_factory=list)
    lock: threading.Lock = field(default_factory=threading.Lock)

    def join(self, hostname: str) -> None:
        with self.lock:
            self.nodes[str(len(self.nodes) + 1)] = hostname

    def handler(self):
        state = self

        class Handler(_Handler):
            def do_POST(self) -> None:
                body = self.read_json()
                if self.path == "/api/v1/preauthkey":
                    with state.lock:
                        state.minted.append(body)
                        key = f"tskey-auth-integration-{len(state.minted)}"
                    self.reply(200, {"preAuthKey": {"key": key}})
                    return
                match = re.fullmatch(r"/api/v1/node/([^/]+)/expire", self.path)
                if match:
                    with state.lock:
                        state.expired.append(state.nodes.get(match.group(1), "?"))
                    self.reply(200, {})
                    return
                self.reply(404, {})

            def do_GET(self) -> None:
                if self.path == "/api/v1/node":
                    with state.lock:
                        nodes = [{"id": k, "givenName": v} for k, v in state.nodes.items()]
                    self.reply(200, {"nodes": nodes})
                    return
                self.reply(404, {})

        return Handler


@dataclass
class FakeS3:
    objects: dict[tuple[str, str], int] = field(default_factory=dict)
    buckets_created: list[str] = field(default_factory=list)
    lock: threading.Lock = field(default_factory=threading.Lock)

    def handler(self):
        state = self

        class Handler(_Handler):
            def do_PUT(self) -> None:
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length) if length else b""
                parts = urllib.parse.unquote(self.path.lstrip("/")).split("/", 1)
                with state.lock:
                    if len(parts) == 1:
                        state.buckets_created.append(parts[0])
                    else:
                        state.objects[(parts[0], parts[1])] = len(body)
                self.reply(200, b"", "application/xml")

            def do_GET(self) -> None:
                parsed = urllib.parse.urlparse(self.path)
                bucket = urllib.parse.unquote(parsed.path.lstrip("/"))
                prefix = urllib.parse.parse_qs(parsed.query).get("prefix", [""])[0]
                with state.lock:
                    rows = [
                        (key, size)
                        for (b, key), size in state.objects.items()
                        if b == bucket and key.startswith(prefix)
                    ]
                contents = "".join(
                    f"<Contents><Key>{key}</Key><Size>{size}</Size></Contents>"
                    for key, size in rows
                )
                self.reply(
                    200,
                    f"<ListBucketResult>{contents}</ListBucketResult>".encode(),
                    "application/xml",
                )

        return Handler


@dataclass
class FakeCloudflare:
    requests: list[dict] = field(default_factory=list)
    auth: list[str] = field(default_factory=list)

    def handler(self):
        state = self

        class Handler(_Handler):
            def do_POST(self) -> None:
                body = self.read_json()
                state.requests.append(body)
                state.auth.append(self.headers.get("Authorization", ""))
                if not self.path.endswith("/r2/temp-access-credentials"):
                    self.reply(404, {"success": False})
                    return
                n = len(state.requests)
                self.reply(
                    200,
                    {
                        "success": True,
                        "result": {
                            "accessKeyId": f"temp-access-{n}",
                            "secretAccessKey": f"temp-secret-{n}",
                            "sessionToken": f"temp-session-{n}",
                        },
                    },
                )

        return Handler


def basic(user: str, password: str) -> str:
    return "Basic " + base64.b64encode(f"{user}:{password}".encode()).decode()


def desktop_handler(agent: "StubAgent"):
    """The guest desktop. Like KasmVNC in session mode it asks for the
    password the guest received in its bootstrap."""

    class Handler(_Handler):
        def do_GET(self) -> None:
            expected = agent.desktop_authorization()
            if expected is None or self.headers.get("Authorization") != expected:
                self.send_response(401)
                self.send_header("WWW-Authenticate", 'Basic realm="desktop"')
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            self.reply(200, f"DCL-DESKTOP {agent.name} path={self.path}".encode(), "text/plain")

    return Handler


@dataclass
class StubAgent:
    """Stands in for one Host Agent and the guest it would boot."""

    name: str
    matchmaker: str
    headscale: FakeHeadscale
    secret: str = field(default_factory=lambda: secrets.token_hex(32))
    mode: str = "ok"  # ok | fail
    boot_seconds: float = 1.0
    host_id: str = ""
    url: str = ""
    desktop_url: str = ""
    provisions: list[dict] = field(default_factory=list)
    refused_bearers: list[str] = field(default_factory=list)
    bootstraps: list[dict] = field(default_factory=list)
    teardowns: int = 0
    beating: bool = True
    pull: bool = False  # True: no listener address is given; it polls instead
    polls: int = 0
    commands: list[str] = field(default_factory=list)

    def desktop_authorization(self) -> str | None:
        """What the desktop of the latest guest on this host asks for."""
        if not self.bootstraps or self.bootstraps[-1]["status"] != 200:
            return None
        body = self.bootstraps[-1]["body"]
        if not body.get("desktopPassword"):
            return None
        return basic(body["desktopUser"], body["desktopPassword"])

    def start(self) -> None:
        _, self.desktop_url = serve(desktop_handler(self))
        _, self.url = serve(self._handler())

    def headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {BOOTSTRAP_TOKEN}", HOST_SECRET_HEADER: self.secret}

    def register(self) -> Reply:
        reply = http(
            "POST",
            f"{self.matchmaker}/v1/hosts",
            headers=self.headers(),
            body={
                "displayName": self.name,
                "labTier": "standard",
                "sharedLogicalCores": 4,
                "sharedMemoryBytes": 8 * 1024**3,
                "shareGpu": False,
                **({} if self.pull else {"agentUrl": self.url}),
            },
        )
        if reply.status == 201:
            self.host_id = reply.json()["hostId"]
        return reply

    def heartbeat(self) -> Reply:
        return http(
            "POST", f"{self.matchmaker}/v1/hosts/{self.host_id}/heartbeat", headers=self.headers()
        )

    def keep_beating(self) -> None:
        def loop() -> None:
            while True:
                if self.beating and self.host_id:
                    try:
                        self.heartbeat()
                    except OSError:
                        pass
                time.sleep(1)

        threading.Thread(target=loop, daemon=True).start()

    def handle(self, path: str, bearer: str, body: dict) -> tuple[int, dict]:
        """What the real agent's provision handler does, for both transports."""
        if bearer != f"Bearer {command_token(self.secret)}":
            self.refused_bearers.append(bearer)
            return 401, {"detail": "unauthorized"}
        if path == "/v1/teardown":
            self.teardowns += 1
            return 200, {"teardown": True}
        if path != "/v1/provision":
            return 404, {}
        self.provisions.append(body)
        time.sleep(self.boot_seconds)
        if self.mode == "fail":
            return 503, {"detail": "QEMU did not start"}
        # Now play the guest: redeem the bootstrap, join the mesh.
        mesh = body.get("guestMesh") or {}
        origin, token = mesh.get("bootstrapOrigin"), mesh.get("bootstrapToken")
        if origin and token:
            redeemed = http(
                "POST",
                f"{origin}/v1/guest/bootstrap",
                headers={"Authorization": f"Bearer {token}"},
            )
            self.bootstraps.append({"status": redeemed.status, "body": redeemed.json()})
            if redeemed.status == 200:
                self.headscale.join(redeemed.json()["hostname"])
        return 200, {"connectionUrl": self.desktop_url + "/", "streamer": "kasm"}

    def _handler(self):
        agent = self

        class Handler(_Handler):
            def do_POST(self) -> None:
                status, reply = agent.handle(
                    self.path, self.headers.get("Authorization", ""), self.read_json()
                )
                self.reply(status, reply)

        return Handler

    def keep_polling(self) -> None:
        """Pull mode: fetch commands like an agent behind a home router."""

        def loop() -> None:
            while True:
                try:
                    polled = http(
                        "POST",
                        f"{self.matchmaker}/v1/hosts/{self.host_id}/commands/poll",
                        headers=self.headers(),
                    )
                    self.polls += 1
                    if polled.status == 200:
                        command = polled.json()
                        self.commands.append(command["kind"])
                        status, reply = self.handle(
                            f"/v1/{command['kind']}",
                            f"Bearer {command['commandToken']}",
                            command.get("body") or {},
                        )
                        http(
                            "POST",
                            f"{self.matchmaker}/v1/hosts/{self.host_id}/commands/"
                            f"{command['commandId']}/result",
                            headers=self.headers(),
                            body={"status": status, "body": json.dumps(reply)},
                        )
                        continue
                except OSError:
                    pass
                time.sleep(0.3)

        threading.Thread(target=loop, daemon=True).start()


# --------------------------------------------------------------------------
# real services


def python_for(component: str) -> str:
    override = os.environ.get(f"{component.upper()}_PYTHON")
    if override:
        return override
    venv = ROOT / component / ".venv" / "bin" / "python"
    return str(venv) if venv.exists() else sys.executable


def database_urls() -> tuple[str, str]:
    base = os.environ.get("INTEGRATION_DATABASE_URL") or os.environ.get("TEST_DATABASE_URL")
    if not base:
        env_file = ROOT / "matchmaker" / ".env"
        if env_file.exists():
            for line in env_file.read_text().splitlines():
                if line.startswith("TEST_DATABASE_URL="):
                    base = line.split("=", 1)[1].strip().strip('"')
    if not base:
        raise SystemExit("Set INTEGRATION_DATABASE_URL or TEST_DATABASE_URL to a Postgres server.")
    head, _, tail = base.rpartition("/")
    query = ""
    if "?" in tail:
        query = "?" + tail.split("?", 1)[1]
    return f"{head}/postgres{query}", f"{head}/{DB_NAME}{query}"


def recreate_database(admin_url: str) -> None:
    code = (
        "import sys\n"
        "from sqlalchemy import create_engine, text\n"
        "engine = create_engine(sys.argv[1], isolation_level='AUTOCOMMIT')\n"
        "with engine.connect() as conn:\n"
        f"    conn.execute(text('DROP DATABASE IF EXISTS {DB_NAME} WITH (FORCE)'))\n"
        f"    conn.execute(text('CREATE DATABASE {DB_NAME}'))\n"
    )
    subprocess.run([python_for("matchmaker"), "-c", code, admin_url], check=True)


def wait_for(url: str, process: subprocess.Popen, log: Path, seconds: float = 40) -> None:
    deadline = time.time() + seconds
    while time.time() < deadline:
        if process.poll() is not None:
            raise SystemExit(f"service exited early. Log:\n{log.read_text()[-4000:]}")
        try:
            if http("GET", url, timeout=2).status < 500:
                return
        except OSError:
            time.sleep(0.3)
    raise SystemExit(f"service did not answer at {url}. Log:\n{log.read_text()[-4000:]}")


def poll_session(matchmaker: str, token: str, session_id: str, until: set[str], seconds: float = 30):
    deadline = time.time() + seconds
    body = {}
    while time.time() < deadline:
        body = http(
            "GET",
            f"{matchmaker}/v1/sessions/{session_id}",
            headers={"Authorization": f"Bearer {token}"},
        ).json()
        if body.get("status") in until:
            return body
        time.sleep(0.4)
    return body


def main() -> int:
    checks = Checks()
    admin_url, db_url = database_urls()
    recreate_database(admin_url)

    headscale, s3, cloudflare = FakeHeadscale(), FakeS3(), FakeCloudflare()
    _, headscale_url = serve(headscale.handler())
    _, s3_url = serve(s3.handler())
    _, cloudflare_url = serve(cloudflare.handler())

    mm_port, gw_port = free_port(), free_port()
    matchmaker = f"http://127.0.0.1:{mm_port}"
    gateway = f"http://127.0.0.1:{gw_port}"
    public_gateway = "https://desktop.integration.invalid"

    logs = Path(tempfile.mkdtemp(prefix="dcl-integration-"))
    mm_env = {
        **os.environ,
        "DATABASE_URL": db_url,
        "JWT_SECRET": secrets.token_hex(32),
        "HOST_BOOTSTRAP_TOKEN": BOOTSTRAP_TOKEN,
        "HOST_APPROVE_TOKEN": APPROVE_TOKEN,
        "HOST_AGENT_PROVISION_URL": "",
        "CREDITS_PER_MINUTE": "1",
        "STALE_SESSION_SWEEP_INTERVAL_SECONDS": "1",
        "HEARTBEAT_STALE_SECONDS": str(STALE_SECONDS),
        "PROVISION_TIMEOUT_SECONDS": "60",
        "IMAGE_STANDARD_DIGEST": STANDARD_DIGEST,
        "IMAGE_STANDARD_SIGNATURE": STANDARD_SIGNATURE,
        "IMAGE_PRO_DIGEST": PRO_DIGEST,
        "IMAGE_PRO_SIGNATURE": PRO_SIGNATURE,
        "HEADSCALE_URL": headscale_url,
        "HEADSCALE_LOGIN_SERVER": headscale_url,
        "HEADSCALE_API_KEY": "integration-headscale-key",
        "HEADSCALE_USER": "1",
        "HEADSCALE_EXIT_NODE": "dcl-platform-exit",
        "S3_ENDPOINT": s3_url,
        "S3_GUEST_ENDPOINT": "",
        "RENTERD_S3_GUEST_ENDPOINT": "",
        "S3_REGION": "auto",
        "S3_ACCESS_KEY": S3_ACCESS_KEY,
        "S3_SECRET_KEY": S3_SECRET_KEY,
        "S3_BUCKET": SHARED_BUCKET,
        "STORAGE_CREDENTIALS": "r2",
        "R2_ACCOUNT_ID": "integration-account",
        "R2_API_TOKEN": R2_API_TOKEN,
        "R2_API_BASE": cloudflare_url,
        "GATEWAY_PUBLIC_ORIGIN": public_gateway,
        "GATEWAY_RESOLVE_TOKEN": RESOLVE_TOKEN,
        "GATEWAY_TOKEN_TTL_SECONDS": "3600",
        "MATCHMAKER_PUBLIC_ORIGIN": matchmaker,
    }
    gw_env = {
        **os.environ,
        "MATCHMAKER_URL": matchmaker,
        "GATEWAY_RESOLVE_TOKEN": RESOLVE_TOKEN,
        "GATEWAY_ALLOW_LOOPBACK_TARGET": "1",
        "GATEWAY_PUBLIC_ORIGIN": "",
        "GATEWAY_LISTEN_PORT": str(gw_port),
    }

    processes: list[subprocess.Popen] = []

    def stop_all() -> None:
        for process in processes:
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
        for process in processes:
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()

    try:
        subprocess.run(
            [python_for("matchmaker"), "-m", "alembic", "upgrade", "head"],
            cwd=ROOT / "matchmaker",
            env=mm_env,
            check=True,
            capture_output=True,
        )
        mm_log, gw_log = logs / "matchmaker.log", logs / "gateway.log"
        mm = subprocess.Popen(
            [python_for("matchmaker"), "-m", "uvicorn", "matchmaker.main:app",
             "--app-dir", "src", "--host", "127.0.0.1", "--port", str(mm_port)],
            cwd=ROOT / "matchmaker", env=mm_env,
            stdout=mm_log.open("w"), stderr=subprocess.STDOUT,
        )
        processes.append(mm)
        gw = subprocess.Popen(
            [python_for("gateway"), "-m", "uvicorn", "dcl_gateway.main:app",
             "--app-dir", "src", "--host", "127.0.0.1", "--port", str(gw_port)],
            cwd=ROOT / "gateway", env=gw_env,
            stdout=gw_log.open("w"), stderr=subprocess.STDOUT,
        )
        processes.append(gw)
        wait_for(f"{matchmaker}/openapi.json", mm, mm_log)
        wait_for(f"{gateway}/s/not-a-real-token-000000000000000000000000000/x", gw, gw_log)

        ok = run_scenarios(checks, matchmaker, gateway, public_gateway, headscale, s3, cloudflare)
        if not ok:
            print(f"\nService logs are in {logs}")
            print("--- matchmaker log (tail) ---")
            print(mm_log.read_text()[-3000:])
    finally:
        stop_all()

    print("\n=== integration summary ===")
    print(f"{checks.passed} checks passed, {len(checks.failed)} failed")
    for label in checks.failed:
        print(f"  FAILED: {label}")
    return 1 if checks.failed else 0


def run_scenarios(checks, matchmaker, gateway, public_gateway, headscale, s3, cloudflare) -> bool:
    check, section = checks.check, checks.section

    def user(email: str) -> tuple[str, str]:
        reply = http(
            "POST",
            f"{matchmaker}/v1/auth/signup",
            body={"email": email, "password": "integration-password"},
        )
        assert reply.status == 201, reply.text
        return reply.json()["accessToken"], reply.json()["user"]["id"]

    def auth(token: str) -> dict[str, str]:
        return {"Authorization": f"Bearer {token}"}

    def book(token: str) -> Reply:
        return http("POST", f"{matchmaker}/v1/sessions", headers=auth(token), body={"labTier": "standard"})

    def approve(host_id: str) -> Reply:
        return http(
            "POST",
            f"{matchmaker}/v1/hosts/{host_id}/approve",
            headers={"Authorization": f"Bearer {APPROVE_TOKEN}"},
        )

    def through_gateway(kasm_url: str) -> tuple[Reply, Reply | None]:
        """Open the public link the way a browser does: link, cookie, page."""
        local = kasm_url.replace(public_gateway, gateway)
        first = http("GET", local)
        cookie = first.headers.get("set-cookie") or first.headers.get("Set-Cookie")
        if first.status != 303 or not cookie:
            return first, None
        page = http("GET", f"{gateway}/", headers={"Cookie": cookie.split(";", 1)[0]})
        return first, page

    # ------------------------------------------------------------------
    section("1. Hosts register, each with its own secret")
    host_a = StubAgent("host-a", matchmaker, headscale, boot_seconds=1.5)
    host_b = StubAgent("host-b", matchmaker, headscale, boot_seconds=0.2)
    for agent in (host_a, host_b):
        agent.start()
        reply = agent.register()
        check(f"{agent.name} registers (201, pending)", reply.status == 201 and reply.json()["status"] == "pending", reply.text)
        check(f"{agent.name} response holds no credential", agent.secret not in reply.text and command_token(agent.secret) not in reply.text)

    token_early, _ = user("early@example.com")
    check("booking before any host is approved -> 409", book(token_early).status == 409)

    for agent in (host_a, host_b):
        check(f"{agent.name} approved", approve(agent.host_id).json().get("status") == "approved")
        check(f"{agent.name} heartbeat with own secret -> 200", agent.heartbeat().status == 200)
        agent.keep_beating()

    stolen = http(
        "POST",
        f"{matchmaker}/v1/hosts/{host_a.host_id}/heartbeat",
        headers={"Authorization": f"Bearer {BOOTSTRAP_TOKEN}", HOST_SECRET_HEADER: host_b.secret},
    )
    check("host-b's secret cannot act as host-a -> 403", stolen.status == 403, stolen.text)
    bare = http(
        "PUT",
        f"{matchmaker}/v1/hosts/{host_a.host_id}",
        headers={"Authorization": f"Bearer {BOOTSTRAP_TOKEN}"},
        body={"displayName": "hijack", "labTier": "standard", "sharedLogicalCores": 1,
              "sharedMemoryBytes": 1, "shareGpu": False, "agentUrl": "http://127.0.0.1:9"},
    )
    check("shared bootstrap token alone cannot re-point host-a -> 403", bare.status == 403, bare.text)

    # ------------------------------------------------------------------
    section("2. Booking returns at once and the desktop boots in the background")
    token_1, user_1 = user("renter-one@example.com")
    started = time.time()
    booked = book(token_1)
    elapsed = time.time() - started
    body = booked.json()
    check("POST /v1/sessions -> 202 provisioning", booked.status == 202 and body.get("status") == "provisioning", booked.text)
    check(f"it returned in {elapsed:.2f}s, before the {host_a.boot_seconds}s boot finished", elapsed < host_a.boot_seconds)
    check("no connection is offered while starting", body.get("connection") is None)
    check("session JSON carries no mesh key", "tskey" not in booked.text and "preauth" not in booked.text.lower())
    session_1 = body["sessionId"]
    again = book(token_1)
    check("asking again returns the same session, still 202", again.status == 202 and again.json()["sessionId"] == session_1)
    ready = poll_session(matchmaker, token_1, session_1, {"active", "failed"})
    check("session becomes active", ready.get("status") == "active", ready)
    chosen = host_a if ready.get("hostId") == host_a.host_id else host_b
    other = host_b if chosen is host_a else host_a
    check("the agent was commanded once, with its own command token", len(chosen.provisions) == 1 and not chosen.refused_bearers, chosen.refused_bearers)
    check("the other agent was not commanded", len(other.provisions) == 0)
    check("only one mesh key was minted, tagged tag:guest", [m.get("aclTags") for m in headscale.minted] == [["tag:guest"]], headscale.minted)

    # ------------------------------------------------------------------
    section("3. The guest gets a storage key for this one user only")
    redeemed = chosen.bootstraps[-1] if chosen.bootstraps else {"status": None, "body": {}}
    storage = (redeemed["body"] or {}).get("storage", {})
    prefix = f"users/{user_1}/files/"
    check("guest bootstrap redeemed -> 200", redeemed["status"] == 200, redeemed)
    check("credential request named the shared bucket and this user's prefix",
          bool(cloudflare.requests) and cloudflare.requests[-1].get("bucket") == SHARED_BUCKET
          and cloudflare.requests[-1].get("prefixes") == [prefix], cloudflare.requests)
    check("credential is read-write, with a lifetime", cloudflare.requests[-1].get("permission") == "object-read-write" and cloudflare.requests[-1].get("ttlSeconds", 0) > 0)
    check("guest received the temporary key and session token", storage.get("accessKeyId", "").startswith("temp-access-") and storage.get("sessionToken", "").startswith("temp-session-"), storage)
    dumped = json.dumps(redeemed["body"])
    check("the master storage key never reached the guest", S3_ACCESS_KEY not in dumped and S3_SECRET_KEY not in dumped)
    check("the Cloudflare API token never reached the guest", R2_API_TOKEN not in dumped)
    check("the provision request held no storage or mesh secret",
          all(s not in json.dumps(chosen.provisions) for s in (S3_SECRET_KEY, "tskey-auth", "temp-secret")))
    check("the user's marker object was written under their prefix", (SHARED_BUCKET, f"{prefix}.dcl/provisioned") in s3.objects, list(s3.objects))
    check("no per-user bucket was created", s3.buckets_created == [])

    # ------------------------------------------------------------------
    section("4. The desktop is reachable through the gateway, and only with the link")
    kasm_url = ready["connection"]["kasmUrl"]
    check("the link is a gateway link, not the host's address", kasm_url.startswith(f"{public_gateway}/s/") and chosen.desktop_url not in json.dumps(ready), kasm_url)
    first, page = through_gateway(kasm_url)
    check("opening the link sets a session cookie (303)", first.status == 303 and page is not None, first.status)
    check("the desktop page is served through the gateway", page is not None and page.status == 200 and f"DCL-DESKTOP {chosen.name}" in page.text, page.text if page else "")
    direct = http("GET", kasm_url.replace(public_gateway, gateway) + "index.html")
    check("the link itself opens nothing but the cookie (no path under it)", direct.status == 404, direct.status)
    replay = http("GET", kasm_url.replace(public_gateway, gateway))
    check("the used link opened by another browser -> 404", replay.status == 404, replay.status)
    page_again = http("GET", f"{gateway}/index.html", headers={"Cookie": first.headers.get("set-cookie", "").split(";", 1)[0]})
    check("a path is proxied with the cookie", page_again.status == 200 and "path=/index.html" in page_again.text, page_again.text)
    desktop_password = (chosen.bootstraps[-1]["body"] or {}).get("desktopPassword") or ""
    check("the guest was given a desktop password", len(desktop_password) == 32, len(desktop_password))
    mesh_peer = http("GET", chosen.desktop_url + "/")
    check("another machine dialling the desktop directly, without the password -> 401", mesh_peer.status == 401, mesh_peer.status)
    check("the renter never sees the desktop password",
          desktop_password not in booked.text and desktop_password not in json.dumps(ready)
          and (page is None or desktop_password not in page.text))
    check("the host's provision request did not carry it", desktop_password not in json.dumps(chosen.provisions))
    guess = http("GET", f"{gateway}/s/{secrets.token_urlsafe(32)}/index.html")
    check("a guessed link -> 404", guess.status == 404)
    no_cookie = http("GET", f"{gateway}/")
    check("no cookie -> no desktop", no_cookie.status in (401, 403, 404) and "DCL-DESKTOP" not in no_cookie.text, no_cookie.status)

    # ------------------------------------------------------------------
    section("5. A second renter gets the other host; a third is told none is free")
    token_2, _ = user("renter-two@example.com")
    booked_2 = book(token_2)
    session_2 = booked_2.json().get("sessionId", "")
    ready_2 = poll_session(matchmaker, token_2, session_2, {"active", "failed"})
    check("second renter is active on the other host", ready_2.get("status") == "active" and ready_2.get("hostId") == other.host_id, ready_2)
    _, page_2 = through_gateway(ready_2["connection"]["kasmUrl"])
    check("second renter reaches their own desktop", page_2 is not None and f"DCL-DESKTOP {other.name}" in page_2.text)
    snoop = http("GET", f"{matchmaker}/v1/sessions/{session_1}", headers=auth(token_2))
    check("renter two cannot read renter one's session -> 404", snoop.status == 404)
    token_3, _ = user("renter-three@example.com")
    third = book(token_3)
    check("third renter -> 409 Host unavailable", third.status == 409 and third.json().get("detail") == "Host unavailable", third.text)

    # ------------------------------------------------------------------
    section("6. Ending a session bills it, stops the guest and kills the link")
    time.sleep(1.2)  # a session shorter than one second bills nothing
    ended = http("POST", f"{matchmaker}/v1/sessions/{session_1}/end", headers=auth(token_1))
    debited = ended.json().get("creditsDebited")
    check("end -> 200 with credits debited", ended.status == 200 and isinstance(debited, int) and debited >= 1, ended.text)
    check("the agent was told to tear the guest down", chosen.teardowns == 1, chosen.teardowns)
    check("the guest's mesh node was expired", f"dcl-guest-{session_1}" in headscale.expired, headscale.expired)
    dead = http("GET", kasm_url.replace(public_gateway, gateway) + "index.html")
    check("the old link no longer opens the desktop -> 404", dead.status == 404, dead.status)
    earnings = http("GET", f"{matchmaker}/v1/hosts/{chosen.host_id}/earnings", headers=chosen.headers())
    check("the host was credited the same amount", earnings.json().get("creditsAccrued") == debited, earnings.text)
    history = http("GET", f"{matchmaker}/v1/sessions", headers=auth(token_1)).json()
    check("history shows the ended session and the debit", history["sessions"][0]["status"] == "ended" and history["creditsDebited"] == debited, history)
    check("no current session afterwards -> 404", http("GET", f"{matchmaker}/v1/sessions/current", headers=auth(token_1)).status == 404)

    # ------------------------------------------------------------------
    section("7. A host that fails to boot gives a clear failure and no charge")
    chosen.mode = "fail"
    failing = book(token_3)
    session_f = failing.json().get("sessionId", "")
    failed = poll_session(matchmaker, token_3, session_f, {"active", "failed"})
    check("session ends as failed with a reason", failed.get("status") == "failed" and bool(failed.get("failureReason")), failed)
    check("no connection on a failed session", failed.get("connection") is None)
    history_3 = http("GET", f"{matchmaker}/v1/sessions", headers=auth(token_3)).json()
    check("the failed session cost nothing", history_3["creditsDebited"] == 0, history_3)
    chosen.mode = "ok"
    retry = book(token_3)
    retried = poll_session(matchmaker, token_3, retry.json().get("sessionId", ""), {"active", "failed"})
    check("the host is free again: retry succeeds", retried.get("status") == "active", retried)
    http("POST", f"{matchmaker}/v1/sessions/{retried.get('sessionId')}/end", headers=auth(token_3))

    # ------------------------------------------------------------------
    section("8. A host that goes silent: the renter is moved to another host")
    # renter two is on `other`; `chosen` is free. Silence `other`.
    other.beating = False
    moved = {}
    deadline = time.time() + STALE_SECONDS + 20
    while time.time() < deadline:
        old = http("GET", f"{matchmaker}/v1/sessions/{session_2}", headers=auth(token_2)).json()
        if old.get("status") == "interrupted":
            current = http("GET", f"{matchmaker}/v1/sessions/current", headers=auth(token_2))
            if current.status == 200 and current.json().get("status") == "active":
                moved = current.json()
                break
        time.sleep(0.5)
    check("the old session is interrupted", old.get("status") == "interrupted", old)
    check("a new session is active on the remaining host", moved.get("hostId") == chosen.host_id and moved.get("sessionId") != session_2, moved)
    if moved:
        _, page_m = through_gateway(moved["connection"]["kasmUrl"])
        check("the renter reaches the new desktop", page_m is not None and f"DCL-DESKTOP {chosen.name}" in page_m.text)
        prefixes = [r.get("prefixes") for r in cloudflare.requests]
        user_2_prefixes = [p for p in prefixes if p != [prefix] and p == prefixes[1]]
        check("the same user's files follow them to the new host", len(user_2_prefixes) >= 2, prefixes)
    history_2 = http("GET", f"{matchmaker}/v1/sessions", headers=auth(token_2)).json()
    check("the interrupted session was billed for the time used", any(s["status"] == "interrupted" for s in history_2["sessions"]), history_2)

    # ------------------------------------------------------------------
    section("9. A host behind a home router fetches its commands itself")
    host_c = StubAgent("host-c", matchmaker, headscale, boot_seconds=0.6, pull=True)
    host_c.start()
    registered = host_c.register()
    check("host-c registers without any address the server could dial",
          registered.status == 201 and registered.json().get("agentUrl") is None, registered.text)
    approve(host_c.host_id)
    host_c.heartbeat()
    host_c.keep_beating()
    spy = http("POST", f"{matchmaker}/v1/hosts/{host_c.host_id}/commands/poll", headers=host_a.headers())
    check("another host cannot read host-c's commands -> 403", spy.status == 403, spy.status)
    shared_only = http("POST", f"{matchmaker}/v1/hosts/{host_c.host_id}/commands/poll",
                       headers={"Authorization": f"Bearer {BOOTSTRAP_TOKEN}"})
    check("the shared bootstrap token alone cannot either -> 403", shared_only.status == 403, shared_only.status)
    host_c.keep_polling()
    time.sleep(0.8)
    check("host-c is polling", host_c.polls >= 1, host_c.polls)
    token_4, _ = user("renter-four@example.com")
    booked_4 = book(token_4)
    session_4 = booked_4.json().get("sessionId", "")
    check("booking is accepted at once (202)", booked_4.status == 202 and booked_4.json().get("hostId") == host_c.host_id, booked_4.text)
    ready_4 = poll_session(matchmaker, token_4, session_4, {"active", "failed"})
    check("the session becomes active through the pulled command", ready_4.get("status") == "active", ready_4)
    check("host-c received exactly one provision command, with its own command token",
          host_c.commands == ["provision"] and len(host_c.provisions) == 1 and not host_c.refused_bearers,
          (host_c.commands, host_c.refused_bearers))
    if ready_4.get("connection"):
        _, page_4 = through_gateway(ready_4["connection"]["kasmUrl"])
        check("the renter reaches the desktop on host-c through the gateway",
              page_4 is not None and "DCL-DESKTOP host-c" in page_4.text)
    time.sleep(1.2)
    ended_4 = http("POST", f"{matchmaker}/v1/sessions/{session_4}/end", headers=auth(token_4))
    check("ending the session works", ended_4.status == 200 and ended_4.json().get("creditsDebited", 0) >= 1, ended_4.text)
    deadline = time.time() + 5
    while time.time() < deadline and host_c.teardowns == 0:
        time.sleep(0.2)
    check("host-c fetched the teardown command and stopped the guest",
          host_c.teardowns == 1 and host_c.commands == ["provision", "teardown"], host_c.commands)

    # ------------------------------------------------------------------
    section("10. Secrets stay where they belong")
    check("every Cloudflare call used the API token as Bearer", all(a == f"Bearer {R2_API_TOKEN}" for a in cloudflare.auth) and len(cloudflare.auth) >= 3, cloudflare.auth)
    check("no agent ever refused the Matchmaker's Bearer", not host_a.refused_bearers and not host_b.refused_bearers)
    wrong = http("POST", f"{host_a.url}/v1/provision", headers={"Authorization": f"Bearer {BOOTSTRAP_TOKEN}"}, body={})
    check("an agent refuses the shared bootstrap token -> 401", wrong.status == 401)

    return not checks.failed


if __name__ == "__main__":
    raise SystemExit(main())
