.PHONY: setup dev build build-mac build-linux build-all dashboard server clean docker copy copy-all

# Install all required tooling (C toolchain, Rust, cargo-watch, Node.js via fnm)
setup:
	@echo "==> Checking C compiler (cc)..."
	@command -v cc >/dev/null 2>&1 || { \
		echo "C compiler not found. Installing build tools..."; \
		if [ "$$(uname)" = "Darwin" ]; then \
			xcode-select --install; \
			echo "Please follow the Xcode CLI tools prompt, then re-run 'make setup'."; \
			exit 1; \
		elif command -v dnf >/dev/null 2>&1; then \
			sudo dnf install -y gcc make pkg-config openssl-devel; \
		elif command -v apt-get >/dev/null 2>&1; then \
			sudo apt-get update && sudo apt-get install -y build-essential pkg-config libssl-dev; \
		elif command -v pacman >/dev/null 2>&1; then \
			sudo pacman -S --noconfirm base-devel openssl; \
		else \
			echo "Unsupported OS. Please install gcc/cc manually."; \
			exit 1; \
		fi; \
	}
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

# Build everything (native)
build: dashboard server

# Build for macOS (Apple Silicon + Intel universal)
build-mac: dashboard
	cargo build --release
	@echo "Binary: target/release/dockyy ($$(ls -lh target/release/dockyy | awk '{print $$5}'))"

# Build for Linux x86_64 (cross-compile via cargo-zigbuild)
build-linux: dashboard
	cargo zigbuild --release --target x86_64-unknown-linux-gnu
	@echo "Binary: target/x86_64-unknown-linux-gnu/release/dockyy ($$(ls -lh target/x86_64-unknown-linux-gnu/release/dockyy | awk '{print $$5}'))"

# Build for both macOS and Linux
build-all: dashboard build-mac build-linux
	@echo "==> All builds complete!"
	@echo "  macOS:  target/release/dockyy"
	@echo "  Linux:  target/x86_64-unknown-linux-gnu/release/dockyy"

# Build dashboard static assets
dashboard:
	cd dashboard && npm install && npm run build

# Build Rust server (release, native)
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

# Copy binaries to binary/{arch} directories
copy:
	@ARCH=$$(uname -m); \
	if [ "$$ARCH" = "arm64" ]; then ARCH="aarch64"; fi; \
	mkdir -p binary/$$ARCH; \
	cp target/release/dockyy binary/$$ARCH/dockyy; \
	echo "Copied to binary/$$ARCH/dockyy"

# Copy all cross-compiled binaries
copy-all:
	@mkdir -p binary/aarch64 binary/x86_64
	@if [ -f target/release/dockyy ]; then \
		cp target/release/dockyy binary/$$(uname -m | sed 's/arm64/aarch64/')/dockyy; \
		echo "Copied native binary to binary/$$(uname -m | sed 's/arm64/aarch64/')/dockyy"; \
	fi
	@if [ -f target/x86_64-unknown-linux-gnu/release/dockyy ]; then \
		cp target/x86_64-unknown-linux-gnu/release/dockyy binary/x86_64/dockyy; \
		echo "Copied linux binary to binary/x86_64/dockyy"; \
	fi

# Clean build artifacts
clean:
	cargo clean
	rm -rf dashboard/dist dashboard/node_modules
