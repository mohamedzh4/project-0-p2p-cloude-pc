from __future__ import annotations

from datetime import datetime
from uuid import UUID

from pydantic import BaseModel, ConfigDict, EmailStr, Field
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
