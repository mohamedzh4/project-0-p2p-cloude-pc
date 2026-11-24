"""setup kits from the website: enrollment codes and host owners

Revision ID: 0013_host_enrollment_codes
Revises: 0012_session_desktop_links
Create Date: 2026-11-23

"""

from __future__ import annotations

from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "0013_host_enrollment_codes"
down_revision: Union[str, None] = "0012_session_desktop_links"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column("hosts", sa.Column("owner_user_id", sa.Uuid(), nullable=True))
    op.create_foreign_key(
        "hosts_owner_user_id_fkey", "hosts", "users", ["owner_user_id"], ["id"],
        ondelete="SET NULL",
    )
    op.create_table(
        "host_enrollment_codes",
        sa.Column("id", sa.Uuid(), nullable=False),
        sa.Column("user_id", sa.Uuid(), nullable=False),
        sa.Column("code_hash", sa.Text(), nullable=False),
        sa.Column("lab_os", sa.Text(), nullable=False),
        sa.Column("expires_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("host_id", sa.Uuid(), nullable=True),
        sa.Column("used_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column(
            "created_at", sa.DateTime(timezone=True), nullable=False,
            server_default=sa.text("now()"),
        ),
        sa.ForeignKeyConstraint(["user_id"], ["users.id"], ondelete="CASCADE"),
        sa.ForeignKeyConstraint(["host_id"], ["hosts.id"], ondelete="CASCADE"),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint("code_hash", name="host_enrollment_codes_code_hash_key"),
    )
    op.create_index("ix_host_enrollment_codes_user_id", "host_enrollment_codes", ["user_id"])


def downgrade() -> None:
    op.drop_index("ix_host_enrollment_codes_user_id", table_name="host_enrollment_codes")
    op.drop_table("host_enrollment_codes")
    op.drop_constraint("hosts_owner_user_id_fkey", "hosts", type_="foreignkey")
    op.drop_column("hosts", "owner_user_id")
