.PHONY: dev build dashboard server clean docker

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
