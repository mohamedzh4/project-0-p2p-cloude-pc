"""per-user renterd S3 bucket (not Storj)

Revision ID: 0005_user_storage
Revises: 0004_ledger_entries
Create Date: 2026-09-05

"""

from __future__ import annotations

from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "0005_user_storage"
down_revision: Union[str, None] = "0004_ledger_entries"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        "user_storage",
        sa.Column("user_id", sa.Uuid(), nullable=False),
        sa.Column("bucket", sa.Text(), nullable=False),
        sa.Column("prefix", sa.Text(), nullable=False),
        sa.Column(
            "created_at",
            sa.DateTime(timezone=True),
            server_default=sa.text("now()"),
            nullable=False,
        ),
        sa.ForeignKeyConstraint(["user_id"], ["users.id"]),
        sa.PrimaryKeyConstraint("user_id"),
        sa.UniqueConstraint("bucket"),
    )


def downgrade() -> None:
    op.drop_table("user_storage")
