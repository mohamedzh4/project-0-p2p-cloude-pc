from __future__ import annotations

from datetime import datetime
from uuid import UUID

from urllib.parse import urlparse

from pydantic import BaseModel, ConfigDict, EmailStr, Field, field_validator, model_validator
from pydantic.alias_generators import to_camel


class CamelModel(BaseModel):
    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
    )


class SignupRequest(BaseModel):
    email: EmailStr
    password: str = Field(min_length=8, max_length=128)


class LoginRequest(BaseModel):
    email: EmailStr
    password: str = Field(min_length=1, max_length=128)


class UserPublic(CamelModel):
    id: UUID
    email: str


class TokenResponse(CamelModel):
    access_token: str
    token_type: str = "bearer"
    user: UserPublic


class SessionRequest(CamelModel):
    lab_tier: str = Field(default="standard", min_length=1, max_length=32)


class MoonlightInfo(CamelModel):
    """Moonlight (GameStream) client hint. Stub host until GATE-PRO."""

    host: str
    app: str = "Desktop"


class ConnectionInfo(CamelModel):
    """Desktop locator. Standard is Kasm; Pro is Sunshine — not Kasm renamed."""

    streamer: str
    kasm_url: str | None = None
    kasm_token: str | None = None
    sunshine_url: str | None = None
    moonlight: MoonlightInfo | None = None
    gpu_exclusive: bool = False


class StorageMountInfo(CamelModel):
    """S3 locators for rclone **inside the guest**. Not for the volunteer host OS.

    Access keys stay in Matchmaker/guest env (`S3_*` / `RENTERD_S3_*`).
    `rclone` is a display string; `rcloneArgs` is the argv a guest image can exec.
    """

    provider: str = "renterd"
    endpoint: str
    region: str
    bucket: str
    prefix: str
    path_style: bool = True
    mount_inside_guest: bool = True
    rclone: str
    rclone_args: list[str] = Field(default_factory=list)
    env_auth: bool = True
    objects: list[str] = Field(default_factory=list)
    saved_as_of: datetime | None = None


class SessionMeshInfo(CamelModel):
    """The guest's mesh identity, for display. Never carries a preauth key."""

    login_server: str
    exit_node: str | None = None
    exit_node_applied: bool = False
    guest_hostname: str


class SessionResponse(CamelModel):
    """`connection` is set only once `status` is `active`.

    `provisioning` means the host is booting the desktop: poll
    `GET /v1/sessions/{id}`. `failed` carries `failureReason`.
    """

    session_id: UUID
    host_id: UUID
    lab_tier: str
    status: str
    failure_reason: str | None = None
    started_at: datetime
    ended_at: datetime | None = None
    connection: ConnectionInfo | None = None
    storage: StorageMountInfo
    mesh: SessionMeshInfo


class EndSessionResponse(CamelModel):
    session_id: UUID
    status: str
    ended_at: datetime
    credits_debited: int


class SessionHistoryItem(CamelModel):
    session_id: UUID
    host_id: UUID
    lab_tier: str
    status: str
    failure_reason: str | None = None
    started_at: datetime
    ended_at: datetime | None = None
    duration_seconds: int | None = None
    credits_debited: int


class SessionHistoryResponse(CamelModel):
    sessions: list[SessionHistoryItem]
    credits_debited: int


class HostEarningsResponse(CamelModel):
    host_id: UUID
    credits_accrued: int


class ImageRegistryResponse(CamelModel):
    lab_tier: str
    digest_sha256: str
    signature: str
    algorithm: str = "ed25519"
    digest_algorithm: str = "sha256"


class HostInventoryRequest(CamelModel):
    display_name: str = Field(min_length=1, max_length=128)
    lab_tier: str = Field(default="standard", min_length=1, max_length=32)
    shared_logical_cores: int = Field(ge=1, le=1024)
    shared_memory_bytes: int = Field(ge=1)
    share_gpu: bool
    shared_gpu_id: str | None = None
    kasm_url: str | None = Field(default=None, max_length=2048)
    agent_url: str | None = Field(default=None, max_length=2048)

    @field_validator("display_name", "lab_tier")
    @classmethod
    def strip_required(cls, value: str) -> str:
        stripped = value.strip()
        if not stripped:
            raise ValueError("value must not be blank")
        return stripped

    @field_validator("shared_gpu_id", "kasm_url")
    @classmethod
    def empty_optional_to_none(cls, value: str | None) -> str | None:
        if value is None:
            return None
        stripped = value.strip()
        return stripped or None

    @field_validator("agent_url")
    @classmethod
    def normalize_agent_url(cls, value: str | None) -> str | None:
        if value is None:
            return None
        stripped = value.strip().rstrip("/")
        if not stripped:
            return None
        parsed = urlparse(stripped)
        if parsed.scheme not in {"http", "https"} or not parsed.netloc:
            raise ValueError("agentUrl must be an http or https URL")
        if parsed.username or parsed.password:
            raise ValueError("agentUrl must not include credentials")
        return stripped

    @model_validator(mode="after")
    def require_named_gpu_when_sharing(self) -> HostInventoryRequest:
        if self.share_gpu and not self.shared_gpu_id:
            raise ValueError(
                "sharedGpuId is required when shareGpu is true; "
                "GPU assignment is exclusive for the session, not a concurrent slice"
            )
        return self


class HostResponse(CamelModel):
    host_id: UUID
    display_name: str
    lab_tier: str
    status: str
    shared_logical_cores: int
    shared_memory_bytes: int
    share_gpu: bool
    shared_gpu_id: str | None = None
    kasm_url: str | None = None
    agent_url: str | None = None
    last_seen_at: datetime | None = None


class MeshPreauthResponse(CamelModel):
    """One-shot Headscale join grant for this Host Agent. `preauthKey` is a secret."""

    login_server: str
    preauth_key: str
    user: str
    reusable: bool
    ephemeral: bool
    expires_at: str


class GuestBootstrapStorage(CamelModel):
    """rclone locators plus S3 env keys. Guest-only — never session JSON."""

    endpoint: str
    region: str
    bucket: str
    prefix: str
    rclone_args: list[str] = Field(default_factory=list)
    env_auth: bool = True
    access_key_id: str
    secret_access_key: str
    # Set for short-lived per-user credentials (Cloudflare R2). The guest
    # exports it as AWS_SESSION_TOKEN.
    session_token: str | None = None
    # True when the user is over quota: the mount is read-only.
    read_only: bool = False


class GuestBootstrapResponse(CamelModel):
    """Secrets the guest image redeems. Not for the volunteer Host Agent."""

    login_server: str
    preauth_key: str
    hostname: str
    exit_node: str | None = None
    acl_tags: list[str]
    tailscale_up: list[str]
    storage: GuestBootstrapStorage
    # The guest sets this as its desktop (KasmVNC) login before it joins the
    # mesh. Only the gateway is told it; renters and hosts never are.
    desktop_user: str | None = None
    desktop_password: str | None = None


class GuestSavedAsOfRequest(CamelModel):
    saved_as_of: datetime | None = None


class GuestSavedAsOfResponse(CamelModel):
    session_id: UUID
    saved_as_of: datetime


class GatewayResolveRequest(CamelModel):
    """Public-gateway lookup. `token` is the path grant, not a user JWT."""

    token: str = Field(min_length=1, max_length=128)


class GatewayRedeemRequest(CamelModel):
    """A desktop link to trade for the session cookie, plus the cookie the
    browser already holds, if any (a used link still works for that browser)."""

    token: str = Field(min_length=1, max_length=128)
    cookie_token: str | None = Field(default=None, max_length=128)


class GatewayResolveResponse(CamelModel):
    """Internal locator for one scoped session. Never a default fallback."""

    session_id: UUID
    target_url: str
    streamer: str
    # Login the gateway presents to the guest desktop on the renter's behalf.
    upstream_user: str | None = None
    upstream_password: str | None = None


class GatewayRedeemResponse(GatewayResolveResponse):
    """Resolve plus the token the gateway sets as the browser's cookie."""

    cookie_token: str


class HostCommandResponse(CamelModel):
    """One instruction for a Host Agent that fetches its commands itself.

    `commandToken` is the same Bearer the agent's own listener expects, so
    the agent can check the command came from the Matchmaker that knows it.
    """

    command_id: UUID
    kind: str
    command_token: str
    body: dict


class HostCommandResultRequest(CamelModel):
    """What the agent's handler answered: its HTTP-style status and body."""

    status: int = Field(ge=100, le=599)
    body: str = Field(default="", max_length=64 * 1024)
