# Database Migrations

This project uses [Alembic](https://alembic.sqlalchemy.org/) for database migrations.

## Quick Start

### Run Migrations

```bash
# Apply all pending migrations
python migrate.py upgrade
```

### Create New Migration

After modifying models in `app/models.py`:

```bash
# Auto-generate migration from model changes
python migrate.py autogenerate -m "Add new field to table"

# Or create empty migration to write manually
python migrate.py revision -m "Custom migration"
```

### View Migration Status

```bash
# Show current migration version
python migrate.py current

# Show migration history
python migrate.py history
```

### Rollback Migrations

```bash
# Rollback one migration
python migrate.py downgrade -1

# Rollback to specific version
python migrate.py downgrade <revision_id>
```

## How It Works

1. **Models** - Define your database schema in `app/models.py`
2. **Generate** - Create migrations with `python migrate.py autogenerate`
3. **Review** - Check the generated migration in `migrations/versions/`
4. **Apply** - Run `python migrate.py upgrade` to apply changes

## Migration Files

- `migrations/` - Alembic configuration and migration scripts
- `migrations/versions/` - Individual migration files (timestamped)
- `alembic.ini` - Alembic configuration file
- `migrate.py` - Helper script for common migration tasks

## Configuration

The database connection is configured via environment variables:
- `TURSO_DATABASE_URL` - Your Turso database URL
- `TURSO_AUTH_TOKEN` - Your Turso authentication token

These are loaded from `.env` file automatically.

## Best Practices

1. **Always review** auto-generated migrations before applying
2. **Test migrations** in development before production
3. **Version control** all migration files
4. **Never modify** applied migrations; create new ones instead
5. **Write descriptive** migration messages

## Example Workflow

```bash
# 1. Modify app/models.py (add/change fields)

# 2. Generate migration
python migrate.py autogenerate -m "Add email field to users"

# 3. Review the generated file in migrations/versions/

# 4. Apply migration
python migrate.py upgrade

# 5. Commit both the model changes and migration file
git add app/models.py migrations/versions/*
git commit -m "Add email field to users"
```
