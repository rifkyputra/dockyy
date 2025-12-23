#!/bin/bash

# Dockyy - Development Server Runner
# This script runs both backend and frontend servers concurrently

set -e

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BLUE}🐳 Starting Dockyy Development Servers...${NC}"

# Function to cleanup background processes on exit
cleanup() {
    echo -e "\n${RED}Shutting down servers...${NC}"
    kill $(jobs -p) 2>/dev/null
    exit
}

trap cleanup SIGINT SIGTERM

# Check if backend venv exists
if [ ! -d "backend/.venv" ]; then
    echo -e "${BLUE}Setting up backend virtual environment...${NC}"
    cd backend
    uv sync
    cd ..
fi

# Check if frontend node_modules exists
if [ ! -d "frontend/node_modules" ]; then
    echo -e "${BLUE}Installing frontend dependencies...${NC}"
    cd frontend
    bun install
    cd ..
fi

# Start backend server
echo -e "${GREEN}Starting Flask backend on http://localhost:8012${NC}"
cd backend
source .venv/bin/activate
python -m app &
BACKEND_PID=$!
cd ..

# Give backend a moment to start
sleep 2

# Start frontend server
echo -e "${GREEN}Starting React frontend on http://localhost:3000${NC}"
cd frontend
SERVER_URL=${SERVER_URL:-"http://localhost:8012"} bun run dev &
FRONTEND_PID=$!
cd ..

echo -e "${BLUE}✨ Both servers are running!${NC}"
echo -e "${GREEN}Frontend: http://localhost:3000${NC}"
echo -e "${GREEN}Backend:  http://localhost:8012${NC}"
echo -e "\nPress Ctrl+C to stop both servers"

# Wait for any process to exit
wait
