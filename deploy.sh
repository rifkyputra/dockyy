#!/usr/bin/env bash
set -euo pipefail

BINARY_SRC="binary/x86_64/dockyy"
BINARY_DST="/usr/local/bin/dockyy"
SERVICE_SRC="dockyy.service"
SERVICE_DST="/etc/systemd/system/dockyy.service"
ENV_DIR="/etc/dockyy"
ENV_FILE="$ENV_DIR/.env"

echo "==> Pulling latest changes..."
git pull

echo "==> Installing binary to $BINARY_DST..."
sudo cp "$BINARY_SRC" "$BINARY_DST"
sudo chmod +x "$BINARY_DST"

echo "==> Installing service file..."
sudo cp "$SERVICE_SRC" "$SERVICE_DST"

# First-time setup: copy .env if not yet in /etc/dockyy
if [ ! -f "$ENV_FILE" ]; then
    if [ -f .env ]; then
        echo "==> First install: copying .env to $ENV_DIR..."
        sudo mkdir -p "$ENV_DIR"
        sudo cp .env "$ENV_FILE"
        sudo chmod 600 "$ENV_FILE"
        echo "    Edit $ENV_FILE to configure your instance."
    else
        echo "WARNING: No .env found. Create $ENV_FILE with your configuration."
    fi
else
    echo "==> Config exists at $ENV_FILE (not overwritten)."
fi

echo "==> Reloading systemd and restarting dockyy..."
sudo systemctl daemon-reload
sudo systemctl enable dockyy
sudo systemctl restart dockyy

echo "==> Done! Status:"
sudo systemctl status dockyy --no-pager
