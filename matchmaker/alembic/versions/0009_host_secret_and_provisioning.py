"""per-host credentials and the provisioning session state

Revision ID: 0009_host_secret_provisioning
Revises: 0008_guest_bootstrap_saved
Create Date: 2026-11-15

"""

from __future__ import annotations

from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "0009_host_secret_provisioning"
down_revision: Union[str, None] = "0008_guest_bootstrap_saved"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None

_OCCUPYING = "status IN ('active', 'provisioning')"
_ACTIVE_ONLY = "status = 'active'"


def _recreate_occupancy_indexes(where: str) -> None:
    op.drop_index("sessions_one_active_per_user", table_name="sessions")
    op.drop_index("sessions_one_active_per_host", table_name="sessions")
    op.create_index(
        "sessions_one_active_per_host",
        "sessions",
        ["host_id"],
        unique=True,
        postgresql_where=sa.text(where),
    )
    op.create_index(
        "sessions_one_active_per_user",
        "sessions",
        ["user_id"],
        unique=True,
        postgresql_where=sa.text(where),
    )


def upgrade() -> None:
    op.add_column("hosts", sa.Column("secret_hash", sa.Text(), nullable=True))
    op.add_column("hosts", sa.Column("command_token", sa.Text(), nullable=True))
    op.add_column("sessions", sa.Column("failure_reason", sa.Text(), nullable=True))
    _recreate_occupancy_indexes(_OCCUPYING)
    # One shared bucket with a per-user prefix (Cloudflare R2) is now allowed.
    op.execute("ALTER TABLE user_storage DROP CONSTRAINT IF EXISTS user_storage_bucket_key")
    op.create_unique_constraint(
        "user_storage_bucket_prefix_key", "user_storage", ["bucket", "prefix"]
    )


def downgrade() -> None:
    op.drop_constraint("user_storage_bucket_prefix_key", "user_storage", type_="unique")
    op.create_unique_constraint("user_storage_bucket_key", "user_storage", ["bucket"])
    op.execute("DELETE FROM sessions WHERE status IN ('provisioning', 'failed')")
    _recreate_occupancy_indexes(_ACTIVE_ONLY)
    op.drop_column("sessions", "failure_reason")
    op.drop_column("hosts", "command_token")
    op.drop_column("hosts", "secret_hash")
