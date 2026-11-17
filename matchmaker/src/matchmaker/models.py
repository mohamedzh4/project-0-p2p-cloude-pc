from __future__ import annotations

from datetime import datetime
from enum import StrEnum
from uuid import UUID, uuid4

from sqlalchemy import (
    BigInteger,
    Boolean,
    DateTime,
    ForeignKey,
    Index,
    Integer,
    Text,
    UniqueConstraint,
    text,
)
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship


class Base(DeclarativeBase):
    pass


class LabTier(StrEnum):
    STANDARD = "standard"
    PRO = "pro"


class HostStatus(StrEnum):
    PENDING = "pending"
    APPROVED = "approved"


class SessionStatus(StrEnum):
    # The host is booked and its agent is booting the guest. Occupies the
    # host and the user the same way `active` does.
    PROVISIONING = "provisioning"
    ACTIVE = "active"
    ENDED = "ended"
    # Silence teardown (host heartbeat older than HEARTBEAT_STALE_AFTER).
    # Partial unique indexes only treat `active` as occupying; interrupted
    # frees the host the same way ended does.
    INTERRUPTED = "interrupted"
    # The agent never returned a desktop. Not billed. `failure_reason` says why.
    FAILED = "failed"


# Statuses that hold a host and a user. Must match the partial unique indexes.
OCCUPYING_STATUSES = (SessionStatus.ACTIVE, SessionStatus.PROVISIONING)
_OCCUPYING_SQL = "status IN ('active', 'provisioning')"


class LedgerParty(StrEnum):
    USER = "user"
    HOST = "host"


class LedgerDirection(StrEnum):
    DEBIT = "debit"
    CREDIT = "credit"


class User(Base):
    __tablename__ = "users"

    id: Mapped[UUID] = mapped_column(primary_key=True, default=uuid4)
    email: Mapped[str] = mapped_column(Text, unique=True, nullable=False)
    password_hash: Mapped[str] = mapped_column(Text, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=text("now()")
    )

    sessions: Mapped[list[LabSession]] = relationship(back_populates="user")
    ledger_entries: Mapped[list[LedgerEntry]] = relationship(back_populates="user")
    storage: Mapped[UserStorage | None] = relationship(back_populates="user")


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
    # Per-host credential (see host_credentials.py). Both are derived from a
    # secret only the Host Agent holds; neither can be turned back into it.
    secret_hash: Mapped[str | None] = mapped_column(Text, nullable=True)
    command_token: Mapped[str | None] = mapped_column(Text, nullable=True)
    # True once this host's agent has asked for commands itself (see
    # host_commands.py). Such a host is never dialled by the Matchmaker.
    pull_mode: Mapped[bool] = mapped_column(
        Boolean, nullable=False, default=False, server_default=text("false")
    )
    status: Mapped[str] = mapped_column(Text, nullable=False)
    last_seen_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=text("now()")
    )

    sessions: Mapped[list[LabSession]] = relationship(back_populates="host")
    ledger_entries: Mapped[list[LedgerEntry]] = relationship(back_populates="host")


class LabSession(Base):
    __tablename__ = "sessions"
    __table_args__ = (
        Index(
            "sessions_one_active_per_host",
            "host_id",
            unique=True,
            postgresql_where=text(_OCCUPYING_SQL),
        ),
        Index(
            "sessions_one_active_per_user",
            "user_id",
            unique=True,
            postgresql_where=text(_OCCUPYING_SQL),
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
    saved_as_of: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    failure_reason: Mapped[str | None] = mapped_column(Text, nullable=True)

    user: Mapped[User] = relationship(back_populates="sessions")
    host: Mapped[Host] = relationship(back_populates="sessions")
    ledger_entries: Mapped[list[LedgerEntry]] = relationship(back_populates="session")
    ingress_tokens: Mapped[list[SessionIngressToken]] = relationship(back_populates="session")
    guest_bootstraps: Mapped[list[SessionGuestBootstrap]] = relationship(
        back_populates="session"
    )


class LedgerEntry(Base):
    """One debit or credit for a settled session. Unique per (session, party)."""

    __tablename__ = "ledger_entries"
    __table_args__ = (
        UniqueConstraint(
            "session_id",
            "party",
            name="ledger_entries_one_per_session_party",
        ),
        Index("ix_ledger_entries_user_id", "user_id"),
        Index("ix_ledger_entries_host_id", "host_id"),
    )

    id: Mapped[UUID] = mapped_column(primary_key=True, default=uuid4)
    session_id: Mapped[UUID] = mapped_column(ForeignKey("sessions.id"), nullable=False)
    user_id: Mapped[UUID | None] = mapped_column(ForeignKey("users.id"), nullable=True)
    host_id: Mapped[UUID | None] = mapped_column(ForeignKey("hosts.id"), nullable=True)
    party: Mapped[str] = mapped_column(Text, nullable=False)
    direction: Mapped[str] = mapped_column(Text, nullable=False)
    amount_credits: Mapped[int] = mapped_column(BigInteger, nullable=False)
    duration_seconds: Mapped[int] = mapped_column(Integer, nullable=False)
    rate_credits_per_minute: Mapped[int] = mapped_column(Integer, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=text("now()")
    )

    session: Mapped[LabSession] = relationship(back_populates="ledger_entries")
    user: Mapped[User | None] = relationship(back_populates="ledger_entries")
    host: Mapped[Host | None] = relationship(back_populates="ledger_entries")


class SessionIngressToken(Base):
    """Scoped public-gateway grant for one active session (architecture §9 item 9).

    The browser presents `token` in the public URL. Only `token_hash` is used
    for lookup. `token` is stored so an existing-session 200 can return the
    same URL. Never a guessable mapping from session id to a guest address.
    """

    __tablename__ = "session_ingress_tokens"
    __table_args__ = (
        UniqueConstraint("session_id", name="session_ingress_tokens_one_per_session"),
        UniqueConstraint("token_hash", name="session_ingress_tokens_token_hash_key"),
        Index("ix_session_ingress_tokens_expires_at", "expires_at"),
    )

    id: Mapped[UUID] = mapped_column(primary_key=True, default=uuid4)
    session_id: Mapped[UUID] = mapped_column(
        ForeignKey("sessions.id", ondelete="CASCADE"), nullable=False
    )
    token_hash: Mapped[str] = mapped_column(Text, nullable=False)
    token: Mapped[str] = mapped_column(Text, nullable=False)
    target_url: Mapped[str] = mapped_column(Text, nullable=False)
    streamer: Mapped[str] = mapped_column(Text, nullable=False)
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    redeemed_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=text("now()")
    )

    session: Mapped[LabSession] = relationship(back_populates="ingress_tokens")


class SessionGuestBootstrap(Base):
    """Guest-only Matchmaker grant (Headscale preauth + rclone locators).

    The volunteer Host Agent receives a bootstrap *token*, never this row's
    `guest_preauth_key` or S3 keys. The guest redeems `POST /v1/guest/bootstrap`.
    """

    __tablename__ = "session_guest_bootstrap"
    __table_args__ = (
        UniqueConstraint("session_id", name="session_guest_bootstrap_one_per_session"),
        UniqueConstraint("token_hash", name="session_guest_bootstrap_token_hash_key"),
        Index("ix_session_guest_bootstrap_expires_at", "expires_at"),
    )

    id: Mapped[UUID] = mapped_column(primary_key=True, default=uuid4)
    session_id: Mapped[UUID] = mapped_column(
        ForeignKey("sessions.id", ondelete="CASCADE"), nullable=False
    )
    token_hash: Mapped[str] = mapped_column(Text, nullable=False)
    login_server: Mapped[str] = mapped_column(Text, nullable=False)
    hostname: Mapped[str] = mapped_column(Text, nullable=False)
    exit_node: Mapped[str | None] = mapped_column(Text, nullable=True)
    acl_tags: Mapped[str] = mapped_column(Text, nullable=False)
    guest_preauth_key: Mapped[str] = mapped_column(Text, nullable=False)
    # Password the guest desktop (KasmVNC) asks for. Known to the guest and
    # the gateway only, so another machine on the mesh cannot open it.
    desktop_password: Mapped[str | None] = mapped_column(Text, nullable=True)
    storage_endpoint: Mapped[str] = mapped_column(Text, nullable=False)
    storage_bucket: Mapped[str] = mapped_column(Text, nullable=False)
    storage_prefix: Mapped[str] = mapped_column(Text, nullable=False)
    storage_region: Mapped[str] = mapped_column(Text, nullable=False)
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    redeemed_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=text("now()")
    )

    session: Mapped[LabSession] = relationship(back_populates="guest_bootstraps")


class HostCommandStatus(StrEnum):
    PENDING = "pending"
    TAKEN = "taken"
    DONE = "done"
    EXPIRED = "expired"


class HostCommand(Base):
    """One instruction waiting for a Host Agent that asks for its commands.

    `payload` can hold a guest bootstrap token, so it is cleared as soon as
    the agent has taken the command.
    """

    __tablename__ = "host_commands"
    __table_args__ = (
        Index("ix_host_commands_host_status_created", "host_id", "status", "created_at"),
    )

    id: Mapped[UUID] = mapped_column(primary_key=True, default=uuid4)
    host_id: Mapped[UUID] = mapped_column(
        ForeignKey("hosts.id", ondelete="CASCADE"), nullable=False
    )
    kind: Mapped[str] = mapped_column(Text, nullable=False)
    payload: Mapped[str | None] = mapped_column(Text, nullable=True)
    status: Mapped[str] = mapped_column(Text, nullable=False)
    result_status: Mapped[int | None] = mapped_column(Integer, nullable=True)
    result_body: Mapped[str | None] = mapped_column(Text, nullable=True)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=text("now()")
    )
    taken_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
    done_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)


class UserStorage(Base):
    """Where one user's files live: a bucket and a prefix inside it.

    Either a bucket per user, or one shared bucket with a per-user prefix.
    """

    __tablename__ = "user_storage"
    __table_args__ = (
        UniqueConstraint("bucket", "prefix", name="user_storage_bucket_prefix_key"),
    )

    user_id: Mapped[UUID] = mapped_column(ForeignKey("users.id"), primary_key=True)
    bucket: Mapped[str] = mapped_column(Text, nullable=False)
    prefix: Mapped[str] = mapped_column(Text, nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True), nullable=False, server_default=text("now()")
    )

    user: Mapped[User] = relationship(back_populates="storage")
