.PHONY: dev setup backend frontend install-pm2 pm2-start pm2-stop pm2-status clean

# Development mode with Make
dev: setup
	@echo "🐳 Starting Dockyy with Make..."
	@$(MAKE) -j2 backend frontend

# Setup dependencies
setup:
	@echo "📦 Checking dependencies..."
	@if [ ! -d "backend/.venv" ]; then \
		echo "Setting up backend virtual environment..."; \
		cd backend && uv sync; \
	fi
	@if [ ! -d "frontend/node_modules" ]; then \
		echo "Installing frontend dependencies..."; \
		cd frontend && bun install; \
	fi

# Run backend server
backend:
	@echo "✨ Starting Flask backend on http://localhost:8012"
	@cd backend && source .venv/bin/activate && python -m app

# Run frontend server
frontend:
	@echo "✨ Starting React frontend on http://localhost:3000"
	@cd frontend && SERVER_URL=http://localhost:8012 bun run dev

# PM2 commands
install-pm2:
	@echo "📦 Installing PM2..."
	@bun install -g pm2

pm2-start: setup
	@echo "🚀 Starting Dockyy with PM2..."
	@pm2 start ecosystem.config.js

pm2-stop:
	@echo "🛑 Stopping Dockyy..."
	@pm2 stop ecosystem.config.js

pm2-restart:
	@echo "🔄 Restarting Dockyy..."
	@pm2 restart ecosystem.config.js

pm2-status:
	@pm2 list

pm2-logs:
	@pm2 logs

pm2-delete:
	@pm2 delete ecosystem.config.js

# Database migrations
migrate:
	@echo "🗄️  Running database migrations..."
	@cd backend && source .venv/bin/activate && python migrate.py

# Clean up
clean:
	@echo "🧹 Cleaning up..."
	@rm -rf backend/.venv
	@rm -rf frontend/node_modules
	@rm -rf backend/__pycache__
	@rm -rf backend/app/__pycache__

# Help
help:
	@echo "Available commands:"
	@echo "  make dev          - Run backend and frontend with Make (parallel)"
	@echo "  make setup        - Install dependencies"
	@echo "  make backend      - Run backend only"
	@echo "  make frontend     - Run frontend only"
	@echo ""
	@echo "PM2 commands:"
	@echo "  make install-pm2  - Install PM2 globally"
	@echo "  make pm2-start    - Start with PM2 (background)"
	@echo "  make pm2-stop     - Stop PM2 processes"
	@echo "  make pm2-restart  - Restart PM2 processes"
	@echo "  make pm2-status   - Show PM2 status"
	@echo "  make pm2-logs     - Show PM2 logs"
	@echo "  make pm2-delete   - Delete PM2 processes"
	@echo ""
	@echo "Other commands:"
	@echo "  make migrate      - Run database migrations"
	@echo "  make clean        - Remove all dependencies"
