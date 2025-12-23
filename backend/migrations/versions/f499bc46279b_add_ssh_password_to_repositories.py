"""add_ssh_password_to_repositories

Revision ID: f499bc46279b
Revises: 14fe94d585ab
Create Date: 2025-12-23 15:05:12.437774

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision: str = 'f499bc46279b'
down_revision: Union[str, Sequence[str], None] = '14fe94d585ab'
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    """Upgrade schema."""
    op.add_column('repositories', sa.Column('ssh_password', sa.String(), nullable=True))


def downgrade() -> None:
    """Downgrade schema."""
    op.drop_column('repositories', 'ssh_password')
