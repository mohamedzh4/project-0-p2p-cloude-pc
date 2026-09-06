"""add hosts.agent_url so Matchmaker can reach the Host Agent

Revision ID: 0003_host_agent_url
Revises: 0002_host_last_seen
Create Date: 2026-09-04

"""

from __future__ import annotations

from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "0003_host_agent_url"
down_revision: Union[str, None] = "0002_host_last_seen"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column(
        "hosts",
        sa.Column("agent_url", sa.Text(), nullable=True),
    )


def downgrade() -> None:
    op.drop_column("hosts", "agent_url")
