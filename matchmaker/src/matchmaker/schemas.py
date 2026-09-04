from __future__ import annotations

from datetime import datetime
from uuid import UUID

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


class ConnectionInfo(CamelModel):
    kasm_url: str
    kasm_token: str | None = None


class SessionResponse(CamelModel):
    session_id: UUID
    host_id: UUID
    lab_tier: str
    started_at: datetime
    connection: ConnectionInfo


class EndSessionResponse(CamelModel):
    session_id: UUID
    status: str
    ended_at: datetime


class HostInventoryRequest(CamelModel):
    display_name: str = Field(min_length=1, max_length=128)
    lab_tier: str = Field(default="standard", min_length=1, max_length=32)
    shared_logical_cores: int = Field(ge=1, le=1024)
    shared_memory_bytes: int = Field(ge=1)
    share_gpu: bool
    shared_gpu_id: str | None = None
    kasm_url: str | None = Field(default=None, max_length=2048)

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
    last_seen_at: datetime | None = None
