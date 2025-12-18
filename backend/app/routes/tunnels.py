from flask import Blueprint, jsonify
import logging
import subprocess

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

    # Register the blueprint
    app.register_blueprint(tunnels_bp)