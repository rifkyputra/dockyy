# Dockyy - Docker Dashboard

A modern Docker dashboard application with a React frontend and Flask backend.

## Architecture

- **Frontend**: React (client-side only, no SSR) with Bun as package manager
- **Backend**: Flask API with UV as package manager and virtual environment

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

- 📊 View all Docker containers
- �️ Manage GitHub repositories
- 🔄 Real-time container status
- 🎨 Modern, responsive UI with DaisyUI
- 🚀 Fast development with Bun and Vite
- 🗄️ Database migrations with Alembic

## API Endpoints

### Health & Containers
- `GET /api/health` - Health check
- `GET /api/containers` - List all containers

### Repositories
- `GET /api/repositories` - List all repositories
- `GET /api/repositories/:id` - Get repository details
- `POST /api/repositories` - Create new repository
- `PUT /api/repositories/:id` - Update repository
- `DELETE /api/repositories/:id` - Delete repository

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

- Frontend uses Vite for fast HMR
- Backend uses Flask with CORS enabled
- Docker Python SDK for container management

## License

MIT

