"""when a pull-mode host last asked for commands

Revision ID: 0011_host_last_polled_at
Revises: 0010_commands_desktop_pw
Create Date: 2026-11-23

"""

from __future__ import annotations

from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "0011_host_last_polled_at"
down_revision: Union[str, None] = "0010_commands_desktop_pw"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # A heartbeat alone does not prove the agent is taking commands (issue 30).
    op.add_column(
        "hosts",
        sa.Column("last_polled_at", sa.DateTime(timezone=True), nullable=True),
    )


def downgrade() -> None:
    op.drop_column("hosts", "last_polled_at")
