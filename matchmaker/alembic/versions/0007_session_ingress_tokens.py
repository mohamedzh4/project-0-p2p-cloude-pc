"""scoped public-gateway tokens per session

Revision ID: 0007_session_ingress_tokens
Revises: 0006_user_storage_bucket
Create Date: 2026-09-08

"""

from __future__ import annotations

from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "0007_session_ingress_tokens"
down_revision: Union[str, None] = "0006_user_storage_bucket"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        "session_ingress_tokens",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("session_id", sa.Uuid(), nullable=False),
        sa.Column("token_hash", sa.Text(), nullable=False),
        sa.Column("token", sa.Text(), nullable=False),
        sa.Column("target_url", sa.Text(), nullable=False),
        sa.Column("streamer", sa.Text(), nullable=False),
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
        sa.UniqueConstraint("session_id", name="session_ingress_tokens_one_per_session"),
        sa.UniqueConstraint("token_hash", name="session_ingress_tokens_token_hash_key"),
    )
    op.create_index(
        "ix_session_ingress_tokens_expires_at",
        "session_ingress_tokens",
        ["expires_at"],
    )


def downgrade() -> None:
    op.drop_index("ix_session_ingress_tokens_expires_at", table_name="session_ingress_tokens")
    op.drop_table("session_ingress_tokens")
