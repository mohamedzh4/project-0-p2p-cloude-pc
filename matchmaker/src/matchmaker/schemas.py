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


class MeshPeerInfo(CamelModel):
    """One Headscale identity for this session. `preauthKey` is a secret."""

    hostname: str
    acl_tags: list[str]
    preauth_key: str
    ephemeral: bool
    reusable: bool
    expires_at: str
    tailscale_up: list[str]


class SessionMeshInfo(CamelModel):
    """Per-session guest+user mesh. Host Agent join is a different route (`tag:host`)."""

    login_server: str
    exit_node: str | None = None
    exit_node_applied: bool = False
    guest: MeshPeerInfo
    user: MeshPeerInfo


class SessionResponse(CamelModel):
    session_id: UUID
    host_id: UUID
    lab_tier: str
    started_at: datetime
    connection: ConnectionInfo
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


class GuestBootstrapResponse(CamelModel):
    """Secrets the guest image redeems. Not for the volunteer Host Agent."""

    login_server: str
    preauth_key: str
    hostname: str
    exit_node: str | None = None
    acl_tags: list[str]
    tailscale_up: list[str]
    storage: GuestBootstrapStorage


class GuestSavedAsOfRequest(CamelModel):
    saved_as_of: datetime | None = None


class GuestSavedAsOfResponse(CamelModel):
    session_id: UUID
    saved_as_of: datetime


class GatewayResolveRequest(CamelModel):
    """Public-gateway lookup. `token` is the path grant, not a user JWT."""

    token: str = Field(min_length=1, max_length=128)


class GatewayResolveResponse(CamelModel):
    """Internal locator for one scoped session. Never a default fallback."""

    session_id: UUID
    target_url: str
    streamer: str
