"""guest bootstrap tokens and session saved-as-of

Revision ID: 0008_guest_bootstrap_saved
Revises: 0007_session_ingress_tokens
Create Date: 2026-09-11

"""

from __future__ import annotations

from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "0008_guest_bootstrap_saved"
down_revision: Union[str, None] = "0007_session_ingress_tokens"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column(
        "sessions",
        sa.Column("saved_as_of", sa.DateTime(timezone=True), nullable=True),
    )
    op.create_table(
        "session_guest_bootstrap",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("session_id", sa.Uuid(), nullable=False),
        sa.Column("token_hash", sa.Text(), nullable=False),
        sa.Column("login_server", sa.Text(), nullable=False),
        sa.Column("hostname", sa.Text(), nullable=False),
        sa.Column("exit_node", sa.Text(), nullable=True),
        sa.Column("acl_tags", sa.Text(), nullable=False),
        sa.Column("guest_preauth_key", sa.Text(), nullable=False),
        sa.Column("storage_endpoint", sa.Text(), nullable=False),
        sa.Column("storage_bucket", sa.Text(), nullable=False),
        sa.Column("storage_prefix", sa.Text(), nullable=False),
        sa.Column("storage_region", sa.Text(), nullable=False),
        sa.Column("expires_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("redeemed_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column(
            "created_at",
            sa.DateTime(timezone=True),
            server_default=sa.text("now()"),
            nullable=False,
        ),
        sa.ForeignKeyConstraint(["session_id"], ["sessions.id"], ondelete="CASCADE"),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint("session_id", name="session_guest_bootstrap_one_per_session"),
        sa.UniqueConstraint("token_hash", name="session_guest_bootstrap_token_hash_key"),
    )
    op.create_index(
        "ix_session_guest_bootstrap_expires_at",
        "session_guest_bootstrap",
        ["expires_at"],
    )


def downgrade() -> None:
    op.drop_index(
        "ix_session_guest_bootstrap_expires_at",
        table_name="session_guest_bootstrap",
    )
    op.drop_table("session_guest_bootstrap")
    op.drop_column("sessions", "saved_as_of")
