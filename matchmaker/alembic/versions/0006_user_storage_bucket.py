"""repair user_storage uniqueness: per-user bucket, shared prefix allowed

Revision ID: 0006_user_storage_bucket
Revises: 0005_user_storage
Create Date: 2026-09-05

0005 initially unique'd `prefix`. Every user uses prefix `files/`, so the
second bucket provision failed. Uniqueness is the per-user bucket name.
This migration is idempotent for both the original and repaired 0005.
"""

from __future__ import annotations

from typing import Sequence, Union

from alembic import op

revision: str = "0006_user_storage_bucket"
down_revision: Union[str, None] = "0005_user_storage"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.execute("ALTER TABLE user_storage DROP CONSTRAINT IF EXISTS user_storage_prefix_key")
    op.execute(
        """
        DO $$ BEGIN
            IF NOT EXISTS (
                SELECT 1 FROM pg_constraint WHERE conname = 'user_storage_bucket_key'
            ) THEN
                ALTER TABLE user_storage ADD CONSTRAINT user_storage_bucket_key UNIQUE (bucket);
            END IF;
        END $$;
        """
    )


def downgrade() -> None:
    op.execute("ALTER TABLE user_storage DROP CONSTRAINT IF EXISTS user_storage_bucket_key")
    op.execute(
        """
        DO $$ BEGIN
            IF NOT EXISTS (
                SELECT 1 FROM pg_constraint WHERE conname = 'user_storage_prefix_key'
            ) THEN
                ALTER TABLE user_storage ADD CONSTRAINT user_storage_prefix_key UNIQUE (prefix);
            END IF;
        END $$;
        """
    )
