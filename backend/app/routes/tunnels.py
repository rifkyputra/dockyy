from flask import Blueprint, jsonify
import logging
import subprocess
import os
import yaml

logger = logging.getLogger(__name__)

tunnels_bp = Blueprint('tunnels', __name__)


def init_routes(app):
    """Register tunnel routes"""

    @tunnels_bp.route('/api/tunnels/cloudflared/status', methods=['GET'])
    def check_cloudflared():
        logger.info("Checking cloudflared installation status")

        try:
            # Check if cloudflared is installed by running version command
            result = subprocess.run(['cloudflared', 'version'], capture_output=True, text=True, timeout=10)
            if result.returncode == 0:
                version = result.stdout.strip()
                logger.info(f"cloudflared is installed: {version}")
                return jsonify({
                    'installed': True,
                    'version': version
                })
            else:
                logger.info("cloudflared is not installed")
                return jsonify({
                    'installed': False,
                    'error': 'cloudflared command not found'
                })
        except subprocess.TimeoutExpired:
            logger.error("cloudflared version check timed out")
            return jsonify({
                'installed': False,
                'error': 'Timeout checking cloudflared'
            }), 500
        except FileNotFoundError:
            logger.info("cloudflared not found")
            return jsonify({
                'installed': False,
                'error': 'cloudflared not found'
            })
        except Exception as e:
            logger.error(f"Error checking cloudflared: {e}")
            return jsonify({
                'installed': False,
                'error': str(e)
            }), 500

    @tunnels_bp.route('/api/tunnels/cloudflared/config', methods=['GET'])
    def get_cloudflared_config():
        logger.info("Getting cloudflared configuration")

        try:
            config_path = os.path.expanduser('~/.cloudflared/config.yaml')
            if os.path.exists(config_path):
                with open(config_path, 'r') as f:
                    config = yaml.safe_load(f)
                return jsonify({
                    'config': config,
                    'config_path': config_path
                })
            else:
                return jsonify({
                    'config': None,
                    'config_path': config_path,
                    'error': 'Config file not found'
                })
        except Exception as e:
            logger.error(f"Error reading cloudflared config: {e}")
            return jsonify({
                'config': None,
                'error': str(e)
            }), 500

    @tunnels_bp.route('/api/tunnels/cloudflared/tunnels', methods=['GET'])
    def list_cloudflared_tunnels():
        logger.info("Listing cloudflared tunnels")

        try:
            result = subprocess.run(['cloudflared', 'tunnel', 'list'], capture_output=True, text=True, timeout=30)
            if result.returncode == 0:
                # Parse the output - cloudflared tunnel list returns JSON
                import json
                tunnels = json.loads(result.stdout)
                return jsonify({
                    'tunnels': tunnels
                })
            else:
                return jsonify({
                    'tunnels': [],
                    'error': result.stderr.strip()
                })
        except subprocess.TimeoutExpired:
            logger.error("cloudflared tunnel list timed out")
            return jsonify({
                'tunnels': [],
                'error': 'Timeout listing tunnels'
            }), 500
        except Exception as e:
            logger.error(f"Error listing cloudflared tunnels: {e}")
            return jsonify({
                'tunnels': [],
                'error': str(e)
            }), 500

    # Register the blueprint
    app.register_blueprint(tunnels_bp)