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

2. Create a virtual environment and install dependencies using UV:
   ```bash
   uv venv
   source .venv/bin/activate  # On Windows: .venv\Scripts\activate
   uv pip install -e ".[dev]"
   ```

3. Copy the environment file:
   ```bash
   cp .env.example .env
   ```

4. Run the Flask server:
   ```bash
   python -m app
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
- 🔄 Real-time container status
- 🎨 Modern, responsive UI
- 🚀 Fast development with Bun and Vite

## API Endpoints

- `GET /api/health` - Health check
- `GET /api/containers` - List all containers

## Development

- Frontend uses Vite for fast HMR
- Backend uses Flask with CORS enabled
- Docker Python SDK for container management

## License

MIT

