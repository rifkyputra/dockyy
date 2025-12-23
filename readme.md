# Dockyy - Docker Dashboard

A modern Docker management dashboard with Git repository integration, Docker Compose project management, and Cloudflare Tunnel support.

## Architecture

- **Frontend**: React (client-side only, no SSR) with TypeScript, Bun, TailwindCSS, DaisyUI, and TanStack Query
- **Backend**: Flask REST API with UV as package manager, SQLAlchemy (Turso/LibSQL), and Alembic for migrations

## Prerequisites

- [Bun](https://bun.sh/) >= 1.0
- [UV](https://github.com/astral-sh/uv) >= 0.1.0
- Python >= 3.11
- Docker Desktop

## Getting Started

### Backend Setup

1. Navigate to the backend directory:
   ```bash
   cd backend
   ```

2. Install dependencies using UV:
   ```bash
   uv sync
   ```

3. Copy the environment file and configure:
   ```bash
   cp .env.example .env
   # Edit .env with your Turso database credentials
   ```

4. Run database migrations:
   ```bash
   python migrate.py upgrade
   ```

5. Run the Flask server:
   ```bash
   uv run python -m app
   ```

The backend will be available at `http://localhost:8012`

### Frontend Setup

1. Navigate to the frontend directory:
   ```bash
   cd frontend
   ```

2. Install dependencies using Bun:
   ```bash
   bun install
   ```

3. Start the development server:
   ```bash
   bun run dev
   ```

The frontend will be available at `http://localhost:3000`

## Features

- 🐳 **Docker Container Management**: View, start, stop, restart, and remove containers
- 📦 **Docker Compose Projects**: Manage multi-container applications with compose up/down/restart
- 📂 **Git Repository Management**: Clone, pull, push, stash, and manage Git repositories with SSH support
- 🔄 **Git Operations**: View file changes, diffs, commit logs, and repository status
- 🌐 **Cloudflare Tunnel Integration**: Manage cloudflared tunnels and configurations
- 🔐 **Authentication**: Simple admin list JWT-based authentication system
- 📊 **Real-time Status**: Live container and project status monitoring
- 📝 **README Viewer**: View repository README files directly in the dashboard
- 🎨 **Modern UI**: Responsive interface built with DaisyUI and TailwindCSS
- ⚡ **Fast Development**: Vite for frontend, Flask for backend

## API Endpoints

### Authentication
- `POST /api/auth/login` - Login and receive JWT token
- `POST /api/auth/verify` - Verify JWT token

### Containers
- `GET /api/containers` - List all containers with status
- `POST /api/containers/<container_id>/start` - Start a container
- `POST /api/containers/<container_id>/stop` - Stop a container
- `POST /api/containers/<container_id>/restart` - Restart a container
- `DELETE /api/containers/<container_id>` - Remove a container
- `GET /api/containers/<container_id>/logs` - Get container logs

### Docker Compose Projects
- `GET /api/projects` - List all Docker Compose projects
- `POST /api/projects/up` - Start a compose project
- `POST /api/projects/down` - Stop and remove a compose project
- `POST /api/projects/restart` - Restart a compose project

### Repositories
- `GET /api/repositories` - List all repositories
- `GET /api/repositories/<id>` - Get repository details
- `POST /api/repositories` - Create/register new repository
- `PUT /api/repositories/<id>` - Update repository
- `DELETE /api/repositories/<id>` - Delete repository
- `GET /api/repositories/<id>/readme` - Get repository README content
- `GET /api/repositories/<id>/status` - Get Git status
- `GET /api/repositories/<id>/log` - Get commit log
- `GET /api/repositories/<id>/diff` - Get file changes
- `POST /api/repositories/<id>/clone` - Clone repository
- `POST /api/repositories/<id>/pull` - Pull latest changes
- `POST /api/repositories/<id>/push` - Push changes to remote
- `POST /api/repositories/<id>/compose` - Check for docker-compose files

### Cloudflare Tunnels
- `GET /api/tunnels/cloudflared/status` - Check cloudflared installation status
- `GET /api/tunnels/cloudflared/config` - Get tunnel configuration
- `POST /api/tunnels/cloudflared/config` - Update tunnel configuration
- `GET /api/tunnels/cloudflared/list` - List all tunnels
- `POST /api/tunnels/cloudflared/start` - Start a tunnel
- `POST /api/tunnels/cloudflared/stop` - Stop a tunnel

## Database Migrations

This project uses Alembic for database schema management. See [backend/migrations/README.md](backend/migrations/README.md) for detailed documentation.

### Common Migration Commands

```bash
# Apply all pending migrations
python migrate.py upgrade

# Create new migration after model changes
python migrate.py autogenerate -m "Description of changes"

# Check current migration version
python migrate.py current

# View migration history
python migrate.py history
```

## Development

### Tech Stack

- **Frontend**: React 18, TypeScript, Vite, TailwindCSS 4, DaisyUI, TanStack Query
- **Backend**: Flask 3, SQLAlchemy with LibSQL (Turso), Alembic, Docker Python SDK, PyYAML
- **Authentication**: JWT (PyJWT)
- **Version Control**: Git operations via subprocess
- **Container Management**: Docker Python SDK

### Environment Variables

Backend (`.env` in backend directory):
```env
DATABASE_URL=libsql://your-turso-url
DATABASE_AUTH_TOKEN=your-turso-token
SECRET_KEY=your-jwt-secret-key
DEFAULT_ADMIN_USERNAME=admin
DEFAULT_ADMIN_PASSWORD=your-secure-password
```

## License

MIT

