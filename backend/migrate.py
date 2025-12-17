#!/usr/bin/env python3
"""
Database migration helper script for Dockyy.
This script provides a clean interface to run Alembic migrations.
"""

import sys
import subprocess
from typing import List


def run_command(cmd: List[str]) -> int:
    """Run a command and return its exit code."""
    try:
        result = subprocess.run(cmd, check=False)
        return result.returncode
    except Exception as e:
        print(f"Error running command: {e}")
        return 1


def main():
    if len(sys.argv) < 2:
        print("Usage: python migrate.py <command> [args]")
        print("\nAvailable commands:")
        print("  upgrade        - Apply all pending migrations")
        print("  downgrade      - Revert migrations (use 'downgrade -1' for one step)")
        print("  current        - Show current migration version")
        print("  history        - Show migration history")
        print("  revision       - Create a new migration (use with -m 'message')")
        print("  autogenerate   - Auto-generate migration from model changes")
        print("\nExamples:")
        print("  python migrate.py upgrade")
        print("  python migrate.py revision -m 'Add new column'")
        print("  python migrate.py autogenerate -m 'Auto migration'")
        return 1

    command = sys.argv[1]
    args = sys.argv[2:]

    if command == "upgrade":
        return run_command(["uv", "run", "alembic", "upgrade", "head"])
    elif command == "downgrade":
        target = args[0] if args else "-1"
        return run_command(["uv", "run", "alembic", "downgrade", target])
    elif command == "current":
        return run_command(["uv", "run", "alembic", "current"])
    elif command == "history":
        return run_command(["uv", "run", "alembic", "history"])
    elif command == "revision":
        return run_command(["uv", "run", "alembic", "revision"] + args)
    elif command == "autogenerate":
        if not args or "-m" not in args:
            print("Error: autogenerate requires -m 'message'")
            print("Example: python migrate.py autogenerate -m 'Add new field'")
            return 1
        return run_command(["uv", "run", "alembic", "revision", "--autogenerate"] + args)
    else:
        print(f"Unknown command: {command}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
