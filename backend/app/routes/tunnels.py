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

    @tunnels_bp.route('/api/tunnels/cloudflared/config', methods=['PUT'])
    def update_cloudflared_config():
        logger.info("Updating cloudflared configuration")

        try:
            from flask import request
            data = request.get_json()
            
            if not data or 'config' not in data:
                return jsonify({
                    'error': 'Config data is required'
                }), 400
            
            config_path = os.path.expanduser('~/.cloudflared/config.yaml')
            
            # Create directory if it doesn't exist
            os.makedirs(os.path.dirname(config_path), exist_ok=True)
            
            # Write the config
            with open(config_path, 'w') as f:
                yaml.dump(data['config'], f, default_flow_style=False)
            
            logger.info(f"Successfully updated cloudflared config at {config_path}")
            
            # Read back the config to confirm
            with open(config_path, 'r') as f:
                updated_config = yaml.safe_load(f)
            
            return jsonify({
                'config': updated_config,
                'config_path': config_path,
                'message': 'Configuration updated successfully'
            })
        except Exception as e:
            logger.error(f"Error updating cloudflared config: {e}")
            return jsonify({
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

    @tunnels_bp.route('/api/tunnels/cloudflared/service/install', methods=['POST'])
    def install_cloudflared_service():
        logger.info("Installing cloudflared service")

        try:
            from flask import request
            data = request.get_json() or {}
            config_path = data.get('config_path', '/home/ubuntu/.cloudflared/config.yaml')
            
            # Run the install command with sudo
            cmd = ['sudo', 'cloudflared', '--config', config_path, 'service', 'install']
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
            
            if result.returncode == 0:
                logger.info(f"Successfully installed cloudflared service")
                return jsonify({
                    'success': True,
                    'message': 'Cloudflared service installed successfully',
                    'stdout': result.stdout,
                    'stderr': result.stderr
                })
            else:
                logger.error(f"Failed to install cloudflared service: {result.stderr}")
                return jsonify({
                    'success': False,
                    'error': result.stderr or 'Failed to install service',
                    'stdout': result.stdout
                }), 500
        except subprocess.TimeoutExpired:
            logger.error("Cloudflared service install timed out")
            return jsonify({
                'success': False,
                'error': 'Timeout installing service'
            }), 500
        except Exception as e:
            logger.error(f"Error installing cloudflared service: {e}")
            return jsonify({
                'success': False,
                'error': str(e)
            }), 500

    @tunnels_bp.route('/api/tunnels/cloudflared/service/uninstall', methods=['POST'])
    def uninstall_cloudflared_service():
        logger.info("Uninstalling cloudflared service")

        try:
            from flask import request
            data = request.get_json() or {}
            config_path = data.get('config_path', '/home/ubuntu/.cloudflared/config.yaml')
            
            # Run the uninstall command with sudo
            cmd = ['sudo', 'cloudflared', '--config', config_path, 'service', 'uninstall']
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
            
            if result.returncode == 0:
                logger.info(f"Successfully uninstalled cloudflared service")
                return jsonify({
                    'success': True,
                    'message': 'Cloudflared service uninstalled successfully',
                    'stdout': result.stdout,
                    'stderr': result.stderr
                })
            else:
                logger.error(f"Failed to uninstall cloudflared service: {result.stderr}")
                return jsonify({
                    'success': False,
                    'error': result.stderr or 'Failed to uninstall service',
                    'stdout': result.stdout
                }), 500
        except subprocess.TimeoutExpired:
            logger.error("Cloudflared service uninstall timed out")
            return jsonify({
                'success': False,
                'error': 'Timeout uninstalling service'
            }), 500
        except Exception as e:
            logger.error(f"Error uninstalling cloudflared service: {e}")
            return jsonify({
                'success': False,
                'error': str(e)
            }), 500

    # Register the blueprint
    app.register_blueprint(tunnels_bp)