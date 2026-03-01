# Dockyy

A blazingly fast, near-zero RAM Docker management dashboard and PaaS.

**Single binary. Embedded database. ~10MB RAM at idle.**

## Architecture

- **Server**: Rust + Axum — single async binary, serves API + embedded dashboard
- **Database**: Embedded SQLite (WAL mode) — no external database needed
- **Dashboard**: Vanilla TypeScript + Vite — compiled to static assets embedded in binary
- **Docker**: bollard SDK — async container management via Unix socket

## Quick Start

```bash
# Install Rust, cargo-watch, and Node.js (skip if already installed)
make setup

# Build everything
make build

# Run
./target/release/dockyy
```

Open `http://localhost:3000` — login with `admin` / `admin`.

### Environment Variables

| Variable             | Default     | Description                 |
| -------------------- | ----------- | --------------------------- |
| `HOST`               | `0.0.0.0`  | Bind address                |
| `PORT`               | `3000`     | Listen port                 |
| `ADMIN_USERNAME`     | `admin`    | Login username              |
| `ADMIN_PASSWORD`     | `admin`    | Login password              |
| `JWT_SECRET`         | (random)   | JWT signing secret          |
| `DOCKYY_DATA_DIR`   | `./data`   | SQLite database directory   |
| `TRAEFIK_HTTP_PORT`  | `80`       | Traefik reverse proxy port  |
| `DISABLE_RATE_LIMIT` | `false`    | Disable login rate limiting |
| `GIT_BIN`            | auto-detect | Path to git binary          |

Create a `.env` file in the working directory (loaded automatically):

```env
PORT=3010
JWT_SECRET=your-secret-here
ADMIN_USERNAME=admin
ADMIN_PASSWORD=your-password
DOCKYY_DATA_DIR=/home/user/dockyy-data
```

## Deployment

### Run as systemd Service (Recommended)

```bash
sudo cp dockyy.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable dockyy
sudo systemctl start dockyy
```

Manage the service:

```bash
sudo systemctl status dockyy      # Check status
sudo systemctl restart dockyy     # Restart after deploy
sudo systemctl stop dockyy        # Stop
sudo journalctl -u dockyy -f      # Tail logs
```

### Deploy Workflow

```bash
# On dev machine
make build-linux && make copy-all
git add binary/x86_64/dockyy
git commit -m "deploy: update binary"
git push

# On server
git pull
sudo systemctl restart dockyy
```

## Features

- 🐳 **Container Management** — start, stop, restart, remove, view logs
- 📂 **Repository Tracking** — register Git repos for deployment
- 🚀 **Push-to-Deploy** — GitHub webhook → automatic build & deploy (via SQLite job queue)
- 🔐 **JWT Authentication** — simple admin auth with argon2
- 📊 **Real-time Dashboard** — modern dark UI with live stats
- ⚡ **Near-Zero RAM** — ~10-20MB idle, no Redis/Postgres required

## API Endpoints

| Method   | Path                            | Description          |
| -------- | ------------------------------- | -------------------- |
| `POST`   | `/api/auth/login`               | Login                |
| `POST`   | `/api/auth/verify`              | Verify JWT           |
| `GET`    | `/api/health`                   | Server health check  |
| `GET`    | `/api/containers`               | List containers      |
| `POST`   | `/api/containers/:id/start`     | Start container      |
| `POST`   | `/api/containers/:id/stop`      | Stop container       |
| `POST`   | `/api/containers/:id/restart`   | Restart container    |
| `DELETE` | `/api/containers/:id`           | Remove container     |
| `GET`    | `/api/containers/:id/logs`      | Container logs       |
| `GET`    | `/api/repositories`             | List repositories    |
| `POST`   | `/api/repositories`             | Create repository    |
| `GET`    | `/api/repositories/:id`         | Get repository       |
| `PUT`    | `/api/repositories/:id`         | Update repository    |
| `DELETE` | `/api/repositories/:id`         | Delete repository    |
| `GET`    | `/api/deployments`              | List deployments     |
| `POST`   | `/api/deployments/:id/redeploy` | Trigger redeployment |
| `POST`   | `/api/webhooks/github`          | GitHub push webhook  |

## Development

```bash
# Dev mode (Vite HMR + Rust server with debug logging)
make dev

# Just the dashboard
cd dashboard && npm run dev

# Just the server
RUST_LOG=dockyy=debug cargo run -p dockyy
```

## Project Structure

```
dockyy/
├── Cargo.toml              # Workspace root
├── Makefile                 # Build commands
├── Dockerfile              # Multi-stage production build
├── crates/server/
│   ├── Cargo.toml          # Server dependencies
│   └── src/
│       ├── main.rs         # Entry point, Axum server setup
│       ├── auth/           # JWT auth + middleware
│       ├── db/             # SQLite database + models
│       ├── routes/         # API route handlers
│       └── services/       # Docker service layer
└── dashboard/
    ├── package.json        # Vite + TypeScript
    ├── src/
    │   ├── main.ts         # SPA entry point
    │   ├── api.ts          # Typed API client
    │   └── style.css       # Design system
    └── dist/               # Built static assets (embedded)
```

## License

MIT
