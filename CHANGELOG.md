# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] - 2026-02-28

### Added
- **Traefik reverse proxy integration** — automatic HTTP routing via a Traefik v3.3 sidecar container on the `dockyy-net` Docker network
- **Per-repository proxy settings** — `domain` and `proxy_port` fields on repositories; containers are launched with Traefik labels when a domain is configured
- **Proxy management UI** — dedicated Proxy page showing Traefik status, active routes, and a one-click start/restart action
- **Environment variable management** — create, update, and delete `.env` variables per repository; injected automatically on `docker-compose up`
- **Import env vars from compose** — parse `environment:` keys from any detected `docker-compose.yml` and import them in one click
- **Swap memory metrics** — health page now reports swap used/total alongside RAM, CPU, and disk

### Changed
- Dashboard sidebar now reflects the current version number
- Health page Resource Usage card includes a Swap progress meter (hidden label on systems with no swap)

## [0.1.0] - 2026-02-01

### Added
- Initial Rust server rewrite (axum 0.8, bollard 0.18, rusqlite bundled) replacing the previous shell-script stack
- Vite/TypeScript dashboard embedded at build time via `rust-embed`
- JWT-based authentication with `bcrypt` password hashing
- Container management — list, start, stop, restart, remove, logs
- Repository management — CRUD, clone, git pull/fetch, docker-compose up
- Deployment job queue with polling worker (5 s interval)
- System health metrics (CPU, RAM, disk) refreshed every 30 s via `sysinfo`
- Webhook endpoint for push-to-deploy automation
- Static file serving with MIME-type detection
