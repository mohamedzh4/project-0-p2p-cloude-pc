from __future__ import annotations

from datetime import datetime
from enum import StrEnum
from uuid import UUID, uuid4

from sqlalchemy import BigInteger, Boolean, DateTime, ForeignKey, Index, Integer, Text, text
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship


class Base(DeclarativeBase):
    pass


class LabTier(StrEnum):
    STANDARD = "standard"


class HostStatus(StrEnum):
    PENDING = "pending"
    APPROVED = "approved"


class SessionStatus(StrEnum):
    ACTIVE = "active"
    ENDED = "ended"


class User(Base):
    __tablename__ = "users"

    id: Mapped[UUID] = mapped_column(primary_key=True, default=uuid4)
    email: Mapped[str] = mapped_column(Text, unique=True, nullable=False)
    password_hash: Mapped[str] = mapped_column(Text, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=text("now()")
    )

    sessions: Mapped[list[LabSession]] = relationship(back_populates="user")


class Host(Base):
    __tablename__ = "hosts"

    id: Mapped[UUID] = mapped_column(primary_key=True, default=uuid4)
    display_name: Mapped[str] = mapped_column(Text, nullable=False)
    lab_tier: Mapped[str] = mapped_column(Text, nullable=False)
    shared_logical_cores: Mapped[int] = mapped_column(Integer, nullable=False)
    shared_memory_bytes: Mapped[int] = mapped_column(BigInteger, nullable=False)
    share_gpu: Mapped[bool] = mapped_column(Boolean, nullable=False)
    shared_gpu_id: Mapped[str | None] = mapped_column(Text, nullable=True)
    kasm_url: Mapped[str] = mapped_column(Text, nullable=False)
    kasm_token: Mapped[str | None] = mapped_column(Text, nullable=True)
    agent_url: Mapped[str | None] = mapped_column(Text, nullable=True)
    status: Mapped[str] = mapped_column(Text, nullable=False)
    last_seen_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=text("now()")
    )

    sessions: Mapped[list[LabSession]] = relationship(back_populates="host")


class LabSession(Base):
    __tablename__ = "sessions"
    __table_args__ = (
        Index(
            "sessions_one_active_per_host",
            "host_id",
            unique=True,
            postgresql_where=text("status = 'active'"),
        ),
        Index(
            "sessions_one_active_per_user",
            "user_id",
            unique=True,
            postgresql_where=text("status = 'active'"),
        ),
    )

    id: Mapped[UUID] = mapped_column(primary_key=True, default=uuid4)
    user_id: Mapped[UUID] = mapped_column(ForeignKey("users.id"), nullable=False)
    host_id: Mapped[UUID] = mapped_column(ForeignKey("hosts.id"), nullable=False)
    status: Mapped[str] = mapped_column(Text, nullable=False)
    started_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=text("now()")
    )
    ended_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)

    user: Mapped[User] = relationship(back_populates="sessions")
    host: Mapped[Host] = relationship(back_populates="sessions")
