"""initial users, hosts, sessions

Revision ID: 0001_initial
Revises:
Create Date: 2026-09-01

"""

from __future__ import annotations

from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "0001_initial"
down_revision: Union[str, None] = None
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        "users",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("email", sa.Text(), nullable=False),
        sa.Column("password_hash", sa.Text(), nullable=False),
        sa.Column(
            "created_at",
            sa.DateTime(timezone=True),
            server_default=sa.text("now()"),
            nullable=False,
        ),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint("email"),
    )
    op.create_table(
        "hosts",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("display_name", sa.Text(), nullable=False),
        sa.Column("lab_tier", sa.Text(), nullable=False),
        sa.Column("shared_logical_cores", sa.Integer(), nullable=False),
        sa.Column("shared_memory_bytes", sa.BigInteger(), nullable=False),
        sa.Column("share_gpu", sa.Boolean(), nullable=False),
        sa.Column("shared_gpu_id", sa.Text(), nullable=True),
        sa.Column("kasm_url", sa.Text(), nullable=False),
        sa.Column("kasm_token", sa.Text(), nullable=True),
        sa.Column("status", sa.Text(), nullable=False),
        sa.Column(
            "created_at",
            sa.DateTime(timezone=True),
            server_default=sa.text("now()"),
            nullable=False,
        ),
        sa.PrimaryKeyConstraint("id"),
    )
    op.create_table(
        "sessions",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("user_id", sa.Uuid(), nullable=False),
        sa.Column("host_id", sa.Uuid(), nullable=False),
        sa.Column("status", sa.Text(), nullable=False),
        sa.Column(
            "started_at",
            sa.DateTime(timezone=True),
            server_default=sa.text("now()"),
            nullable=False,
        ),
        sa.Column("ended_at", sa.DateTime(timezone=True), nullable=True),
        sa.ForeignKeyConstraint(["host_id"], ["hosts.id"]),
        sa.ForeignKeyConstraint(["user_id"], ["users.id"]),
        sa.PrimaryKeyConstraint("id"),
    )
    op.create_index(
        "sessions_one_active_per_host",
        "sessions",
        ["host_id"],
        unique=True,
        postgresql_where=sa.text("status = 'active'"),
    )
    op.create_index(
        "sessions_one_active_per_user",
        "sessions",
        ["user_id"],
        unique=True,
        postgresql_where=sa.text("status = 'active'"),
    )


def downgrade() -> None:
    op.drop_index("sessions_one_active_per_user", table_name="sessions")
    op.drop_index("sessions_one_active_per_host", table_name="sessions")
    op.drop_table("sessions")
    op.drop_table("hosts")
    op.drop_table("users")
