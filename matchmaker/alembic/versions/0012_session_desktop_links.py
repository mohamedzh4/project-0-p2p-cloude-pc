"""single-use desktop links

Revision ID: 0012_session_desktop_links
Revises: 0011_host_last_polled_at
Create Date: 2026-11-23

"""

from __future__ import annotations

from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "0012_session_desktop_links"
down_revision: Union[str, None] = "0011_host_last_polled_at"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        "session_desktop_links",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("session_id", sa.Uuid(), nullable=False),
        sa.Column("token_hash", sa.Text(), nullable=False),
        sa.Column("expires_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("used_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column(
            "created_at",
            sa.DateTime(timezone=True),
            nullable=False,
            server_default=sa.text("now()"),
        ),
        sa.ForeignKeyConstraint(["session_id"], ["sessions.id"], ondelete="CASCADE"),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint("token_hash", name="session_desktop_links_token_hash_key"),
    )
    op.create_index(
        "ix_session_desktop_links_session_id", "session_desktop_links", ["session_id"]
    )


def downgrade() -> None:
    op.drop_index("ix_session_desktop_links_session_id", table_name="session_desktop_links")
    op.drop_table("session_desktop_links")
