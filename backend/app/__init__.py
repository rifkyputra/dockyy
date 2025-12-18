from flask import Flask, jsonify
from flask_cors import CORS
import docker
import logging
import os
from sqlalchemy import create_engine
from dotenv import load_dotenv
from app.models import Base
from app.routes.repositories import init_routes as init_repositories

# Set up logging first
logging.basicConfig(level=logging.DEBUG)
logger = logging.getLogger(__name__)

load_dotenv()

# Get environment variables
TURSO_DATABASE_URL = os.environ.get("TURSO_DATABASE_URL")
TURSO_AUTH_TOKEN = os.environ.get("TURSO_AUTH_TOKEN")


# Validate required environment variables
if not TURSO_DATABASE_URL:
    raise ValueError("TURSO_DATABASE_URL environment variable is required")
if not TURSO_AUTH_TOKEN:
    raise ValueError("TURSO_AUTH_TOKEN environment variable is required")

# Debug logging
logger.info(f"TURSO_DATABASE_URL: {TURSO_DATABASE_URL}")
logger.info(f"TURSO_AUTH_TOKEN: {TURSO_AUTH_TOKEN}")

# Construct SQLAlchemy URL for Turso (note the /? before query params)
dbUrl = f"sqlite+{TURSO_DATABASE_URL}?secure=true"
logger.info(f"Database URL constructed (+{dbUrl})")

# Create engine
dbEngine = create_engine(
    f"sqlite+{TURSO_DATABASE_URL}?secure=true",
    connect_args={
        "auth_token": TURSO_AUTH_TOKEN,
    },
    echo=True  # Enable SQL query logging for debugging
)

# Note: Database migrations are managed via Alembic.
# To create/update tables, run: python migrate.py upgrade
# To create new migrations, run: python migrate.py autogenerate -m "description"
logger.info("Database engine initialized. Use Alembic for migrations.")

app = Flask(__name__)

# Configure CORS to allow all origins for development
CORS(app, resources={
    r"/*": {
        "origins": "*",
        "methods": ["GET", "POST", "PUT", "DELETE", "OPTIONS"],
        "allow_headers": ["Content-Type", "Authorization"]
    }
})

# Set up logging
logging.basicConfig(level=logging.DEBUG)
logger = logging.getLogger(__name__)

from app.routes.containers import init_routes as init_containers
from app.routes.tunnels import init_routes as init_tunnels


# Initialize Docker client with error handling
try:
    docker_client = docker.from_env()
    logger.info("Docker client initialized successfully")
except Exception as e:
    logger.error(f"Failed to initialize Docker client: {e}")
    docker_client = None

# Initialize  routes
init_repositories(app, dbEngine)
init_containers(app, docker_client)
init_tunnels(app)

@app.route('/api/health', methods=['GET'])
def health():
    logger.info("Health check endpoint called")
    docker_status = "connected" if docker_client else "disconnected"
    return jsonify({
        'status': 'ok', 
        'message': 'Dockyy API is running',
        'docker': docker_status
    })

@app.route('/api/readme', methods=['GET'])
def get_readme():
    """Get the project's README.md content"""
    try:
        # Get the project root (parent directory of backend/)
        project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        readme_path = os.path.join(project_root, 'readme.md')
        
        if not os.path.exists(readme_path):
            return jsonify({'error': 'README not found'}), 404
        
        with open(readme_path, 'r', encoding='utf-8') as f:
            content = f.read()
        
        return jsonify({'content': content}), 200
    except Exception as e:
        logger.error(f"Error reading README: {e}")
        return jsonify({'error': str(e)}), 500
