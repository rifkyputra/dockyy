from flask import Flask, jsonify
from flask_cors import CORS
import docker
import logging

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
            container_list.append({
                'id': container.id[:12],
                'name': container.name,
                'status': container.status,
                'image': container.image.tags[0] if container.image.tags else 'unknown'
            })
        logger.info(f"Found {len(container_list)} containers")
        return jsonify(container_list)
    except Exception as e:
        logger.error(f"Error fetching containers: {e}")
        return jsonify({'error': str(e)}), 500
