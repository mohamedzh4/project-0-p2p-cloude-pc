"""host command queue and per-session desktop password

Revision ID: 0010_commands_desktop_pw
Revises: 0009_host_secret_provisioning
Create Date: 2026-11-16

"""

from __future__ import annotations

from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "0010_commands_desktop_pw"
down_revision: Union[str, None] = "0009_host_secret_provisioning"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # The guest desktop asks for this password; only the gateway knows it.
    op.add_column(
        "session_guest_bootstrap",
        sa.Column("desktop_password", sa.Text(), nullable=True),
    )
    # True once a Host Agent has asked for commands itself (it is behind a
    # home router and the Matchmaker cannot dial it).
    op.add_column(
        "hosts",
        sa.Column("pull_mode", sa.Boolean(), nullable=False, server_default=sa.false()),
    )
    op.create_table(
        "host_commands",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("host_id", sa.Uuid(), nullable=False),
        sa.Column("kind", sa.Text(), nullable=False),
        sa.Column("payload", sa.Text(), nullable=True),
        sa.Column("status", sa.Text(), nullable=False),
        sa.Column("result_status", sa.Integer(), nullable=True),
        sa.Column("result_body", sa.Text(), nullable=True),
        sa.Column(
            "created_at",
            sa.DateTime(timezone=True),
            server_default=sa.text("now()"),
            nullable=False,
        ),
        sa.Column("taken_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("done_at", sa.DateTime(timezone=True), nullable=True),
        sa.ForeignKeyConstraint(["host_id"], ["hosts.id"], ondelete="CASCADE"),
        sa.PrimaryKeyConstraint("id"),
    )
    op.create_index(
        "ix_host_commands_host_status_created",
        "host_commands",
        ["host_id", "status", "created_at"],
    )


def downgrade() -> None:
    op.drop_index("ix_host_commands_host_status_created", table_name="host_commands")
    op.drop_table("host_commands")
    op.drop_column("hosts", "pull_mode")
    op.drop_column("session_guest_bootstrap", "desktop_password")
