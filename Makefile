.PHONY: setup dev build dashboard server clean docker

# Install all required tooling (Rust, cargo-watch, Node.js via fnm)
setup:
	@echo "==> Checking Rust toolchain..."
	@command -v rustup >/dev/null 2>&1 || { \
		echo "Installing Rust via rustup..."; \
		curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y; \
		. "$$HOME/.cargo/env"; \
	}
	@echo "==> Ensuring stable toolchain..."
	@rustup toolchain install stable
	@rustup default stable
	@echo "==> Installing cargo-watch..."
	@command -v cargo-watch >/dev/null 2>&1 || cargo install cargo-watch
	@echo "==> Checking Node.js..."
	@command -v node >/dev/null 2>&1 || { \
		echo "Node.js not found. Installing via fnm..."; \
		command -v fnm >/dev/null 2>&1 || curl -fsSL https://fnm.vercel.app/install | bash; \
		eval "$$(fnm env)"; \
		fnm install --lts; \
	}
	@echo "==> Setup complete!"
	@echo "    rust: $$(rustc --version)"
	@echo "    cargo: $$(cargo --version)"
	@echo "    node: $$(node --version)"

run:
	make build; pkill dockyy; sleep 2; ./target/release/dockyy

# Development: run Rust server (auto-reload) + Vite dev server (HMR)
dev:
	@echo "Starting dashboard dev server..."
	cd dashboard && npm run dev &
	@echo "Starting Rust server with live reload..."
	RUST_LOG=dockyy=debug cargo watch -x run -w crates/server/src -i dashboard

# Build everything
build: dashboard server

# Build dashboard static assets
dashboard:
	cd dashboard && npm install && npm run build

# Build Rust server (release)
server: dashboard
	cargo build --release
	@echo "Binary: target/release/dockyy ($$(ls -lh target/release/dockyy | awk '{print $$5}'))"

# Build Docker image
docker:
	docker build -t dockyy:latest .

# Run in Docker (mounting host Docker socket)
docker-run:
	docker run -d \
		--name dockyy \
		-p 3000:3000 \
		-v /var/run/docker.sock:/var/run/docker.sock \
		-v dockyy_data:/data \
		-e ADMIN_USERNAME=admin \
		-e ADMIN_PASSWORD=admin \
		-e JWT_SECRET=change-me \
		dockyy:latest

# Clean build artifacts
clean:
	cargo clean
	rm -rf dashboard/dist dashboard/node_modules
