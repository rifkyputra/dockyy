from flask import Flask, jsonify
from flask_cors import CORS
import docker
import logging
import os
from sqlalchemy import create_engine
from dotenv import load_dotenv
from app.models import Base
from app.routes.repositories import init_routes

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

# Initialize Docker client with error handling
try:
    docker_client = docker.from_env()
    logger.info("Docker client initialized successfully")
except Exception as e:
    logger.error(f"Failed to initialize Docker client: {e}")
    docker_client = None

# Initialize repository routes
init_routes(app, dbEngine)

@app.route('/api/health', methods=['GET'])
def health():
    logger.info("Health check endpoint called")
    docker_status = "connected" if docker_client else "disconnected"
    return jsonify({
        'status': 'ok', 
        'message': 'Dockyy API is running',
        'docker': docker_status
    })

@app.route('/api/containers', methods=['GET'])
def get_containers():
    logger.info("Containers endpoint called")
    
    if not docker_client:
        logger.error("Docker client not available")
        return jsonify({'error': 'Docker client not initialized. Is Docker running?'}), 503
    
    try:
        containers = docker_client.containers.list(all=True)
        container_list = []
        for container in containers:
            # Extract docker compose project label if it exists
            compose_project = container.labels.get('com.docker.compose.project', None)
            container_list.append({
                'id': container.id[:12],
                'name': container.name,
                'status': container.status,
                'image': container.image.tags[0] if container.image.tags else 'unknown',
                'compose_project': compose_project
            })
        logger.info(f"Found {len(container_list)} containers")
        return jsonify(container_list)
    except Exception as e:
        logger.error(f"Error fetching containers: {e}")
        return jsonify({'error': str(e)}), 500

@app.route('/api/containers/<container_id>/start', methods=['POST'])
def start_container(container_id):
    logger.info(f"Start container endpoint called for {container_id}")
    
    if not docker_client:
        logger.error("Docker client not available")
        return jsonify({'error': 'Docker client not initialized. Is Docker running?'}), 503
    
    try:
        container = docker_client.containers.get(container_id)
        container.start()
        logger.info(f"Container {container_id} started successfully")
        return jsonify({
            'success': True,
            'message': f'Container {container.name} started successfully'
        })
    except docker.errors.NotFound:
        logger.error(f"Container {container_id} not found")
        return jsonify({'error': 'Container not found'}), 404
    except Exception as e:
        logger.error(f"Error starting container {container_id}: {e}")
        return jsonify({'error': str(e)}), 500

@app.route('/api/containers/<container_id>/stop', methods=['POST'])
def stop_container(container_id):
    logger.info(f"Stop container endpoint called for {container_id}")
    
    if not docker_client:
        logger.error("Docker client not available")
        return jsonify({'error': 'Docker client not initialized. Is Docker running?'}), 503
    
    try:
        container = docker_client.containers.get(container_id)
        container.stop()
        logger.info(f"Container {container_id} stopped successfully")
        return jsonify({
            'success': True,
            'message': f'Container {container.name} stopped successfully'
        })
    except docker.errors.NotFound:
        logger.error(f"Container {container_id} not found")
        return jsonify({'error': 'Container not found'}), 404
    except Exception as e:
        logger.error(f"Error stopping container {container_id}: {e}")
        return jsonify({'error': str(e)}), 500

@app.route('/api/projects/<project_name>/start', methods=['POST'])
def start_project(project_name):
    logger.info(f"Start project endpoint called for {project_name}")
    
    if not docker_client:
        logger.error("Docker client not available")
        return jsonify({'error': 'Docker client not initialized. Is Docker running?'}), 503
    
    try:
        # Get all containers for this project
        containers = docker_client.containers.list(
            all=True,
            filters={'label': f'com.docker.compose.project={project_name}'}
        )
        
        if not containers:
            return jsonify({'error': f'No containers found for project {project_name}'}), 404
        
        started_count = 0
        errors = []
        
        for container in containers:
            try:
                if container.status != 'running':
                    container.start()
                    started_count += 1
                    logger.info(f"Started container {container.name}")
            except Exception as e:
                error_msg = f"Error starting {container.name}: {str(e)}"
                logger.error(error_msg)
                errors.append(error_msg)
        
        logger.info(f"Started {started_count} containers for project {project_name}")
        return jsonify({
            'success': True,
            'message': f'Started {started_count} container(s) for project {project_name}',
            'started_count': started_count,
            'errors': errors if errors else None
        })
    except Exception as e:
        logger.error(f"Error starting project {project_name}: {e}")
        return jsonify({'error': str(e)}), 500

@app.route('/api/projects/<project_name>/stop', methods=['POST'])
def stop_project(project_name):
    logger.info(f"Stop project endpoint called for {project_name}")
    
    if not docker_client:
        logger.error("Docker client not available")
        return jsonify({'error': 'Docker client not initialized. Is Docker running?'}), 503
    
    try:
        # Get all containers for this project
        containers = docker_client.containers.list(
            all=True,
            filters={'label': f'com.docker.compose.project={project_name}'}
        )
        
        if not containers:
            return jsonify({'error': f'No containers found for project {project_name}'}), 404
        
        stopped_count = 0
        errors = []
        
        for container in containers:
            try:
                if container.status == 'running':
                    container.stop()
                    stopped_count += 1
                    logger.info(f"Stopped container {container.name}")
            except Exception as e:
                error_msg = f"Error stopping {container.name}: {str(e)}"
                logger.error(error_msg)
                errors.append(error_msg)
        
        logger.info(f"Stopped {stopped_count} containers for project {project_name}")
        return jsonify({
            'success': True,
            'message': f'Stopped {stopped_count} container(s) for project {project_name}',
            'stopped_count': stopped_count,
            'errors': errors if errors else None
        })
    except Exception as e:
        logger.error(f"Error stopping project {project_name}: {e}")
        return jsonify({'error': str(e)}), 500


@app.route('/api/projects/<project_name>/restart', methods=['POST'])
def restart_project(project_name):
    logger.info(f"Restart project endpoint called for {project_name}")

    if not docker_client:
        logger.error("Docker client not available")
        return jsonify({'error': 'Docker client not initialized. Is Docker running?'}), 503

    try:
        # Stop then start
        containers = docker_client.containers.list(
            all=True,
            filters={'label': f'com.docker.compose.project={project_name}'}
        )

        if not containers:
            return jsonify({'error': f'No containers found for project {project_name}'}), 404

        stopped_count = 0
        started_count = 0
        errors = []

        # Stop
        for container in containers:
            try:
                if container.status == 'running':
                    container.stop()
                    stopped_count += 1
                    logger.info(f"Stopped container {container.name} for restart")
            except Exception as e:
                err = f"Error stopping {container.name}: {e}"
                logger.error(err)
                errors.append(err)

        # refresh containers list
        containers = docker_client.containers.list(
            all=True,
            filters={'label': f'com.docker.compose.project={project_name}'}
        )

        for container in containers:
            try:
                if container.status != 'running':
                    container.start()
                    started_count += 1
                    logger.info(f"Started container {container.name} after restart")
            except Exception as e:
                err = f"Error starting {container.name}: {e}"
                logger.error(err)
                errors.append(err)

        return jsonify({'success': True, 'stopped': stopped_count, 'started': started_count, 'errors': errors if errors else None})
    except Exception as e:
        logger.error(f"Error restarting project {project_name}: {e}")
        return jsonify({'error': str(e)}), 500


@app.route('/api/projects/<project_name>/rebuild', methods=['POST'])
def rebuild_project(project_name):
    """Rebuilds a compose project by running `docker compose up -d --build --force-recreate` in the provided path.
    Request body JSON: { "path": "/absolute/path/to/project" }
    """
    import subprocess
    from flask import request

    logger.info(f"Rebuild project endpoint called for {project_name}")

    data = request.get_json() or {}
    path = data.get('path')

    if not path or not os.path.exists(path):
        logger.error(f"Invalid or missing path for rebuild: {path}")
        return jsonify({'error': 'Missing or invalid path in request body'}), 400

    # run docker compose up with build
    try:
        cmd = ['docker', 'compose', 'up', '-d', '--build', '--force-recreate']
        proc = subprocess.run(cmd, cwd=path, capture_output=True, text=True, timeout=300)
        return jsonify({'returncode': proc.returncode, 'stdout': proc.stdout, 'stderr': proc.stderr}), 200
    except subprocess.TimeoutExpired as e:
        logger.error(f"Rebuild timeout for {project_name} at {path}: {e}")
        return jsonify({'error': 'Rebuild timed out', 'details': str(e)}), 500
    except Exception as e:
        logger.error(f"Error rebuilding project {project_name} at {path}: {e}")
        return jsonify({'error': str(e)}), 500
