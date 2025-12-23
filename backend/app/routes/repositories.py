from flask import Blueprint, jsonify, request
from sqlalchemy import select
from sqlalchemy.orm import Session
from app.models import Repository
from datetime import datetime
import logging
import os
import docker
from app.services.git_service import (
    check_git_repo,
    run_git_cmd,
    parse_unified_diff,
)
from app.services.compose_service import check_docker_compose

logger = logging.getLogger(__name__)



repositories_bp = Blueprint('repositories', __name__)

def init_routes(app, db_engine):
    """Initialize repository routes with database engine"""
    
    @repositories_bp.route('/api/repositories', methods=['GET'])
    def get_repositories():
        """Get all repositories"""
        try:
            session = Session(db_engine)
            stmt = select(Repository)
            repos_list = []
            
            for repo in session.scalars(stmt):
                repos_list.append({
                    'id': repo.id,
                    'name': repo.name,
                    'owner': repo.owner,
                    'url': repo.url,
                    'description': repo.description,
                    'webhook_url': repo.webhook_url,
                    'filesystem_path': repo.filesystem_path,
                    'ssh_password': repo.ssh_password,
                    'is_private': repo.is_private,
                    'default_branch': repo.default_branch,
                    'created_at': repo.created_at.isoformat() if repo.created_at else None,
                    'updated_at': repo.updated_at.isoformat() if repo.updated_at else None
                })
            
            session.close()
            logger.info(f"Retrieved {len(repos_list)} repositories")
            return jsonify(repos_list), 200
        except Exception as e:
            logger.error(f"Error fetching repositories: {e}")
            return jsonify({'error': str(e)}), 500

    @repositories_bp.route('/api/repositories/<int:repo_id>', methods=['GET'])
    def get_repository(repo_id):
        """Get a specific repository by ID"""
        try:
            session = Session(db_engine)
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)
            
            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404
            
            repo_dict = {
                'id': repo.id,
                'name': repo.name,
                'owner': repo.owner,
                'url': repo.url,
                'description': repo.description,
                'webhook_url': repo.webhook_url,
                'filesystem_path': repo.filesystem_path,
                'is_private': repo.is_private,
                'default_branch': repo.default_branch,
                'created_at': repo.created_at.isoformat() if repo.created_at else None,
                'updated_at': repo.updated_at.isoformat() if repo.updated_at else None
            }
            
            session.close()
            logger.info(f"Retrieved repository {repo_id}")
            return jsonify(repo_dict), 200
        except Exception as e:
            logger.error(f"Error fetching repository {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500

    @repositories_bp.route('/api/repositories', methods=['POST'])
    def create_repository():
        """Create a new repository"""
        try:
            data = request.get_json()
            
            # Validate required fields
            required_fields = ['name', 'owner', 'url']
            for field in required_fields:
                if field not in data:
                    return jsonify({'error': f'Missing required field: {field}'}), 400
            
            session = Session(db_engine)
            
            # Create new repository object
            new_repo = Repository(
                name=data['name'],
                owner=data['owner'],
                url=data['url'],
                description=data.get('description'),
                webhook_url=data.get('webhook_url'),
                filesystem_path=data.get('filesystem_path'),
                ssh_password=data.get('ssh_password'),
                is_private=data.get('is_private', False),
                default_branch=data.get('default_branch', 'main'),
                created_at=datetime.utcnow(),
                updated_at=datetime.utcnow()
            )
            
            session.add(new_repo)
            session.commit()
            session.refresh(new_repo)
            
            repo_dict = {
                'id': new_repo.id,
                'name': new_repo.name,
                'owner': new_repo.owner,
                'url': new_repo.url,
                'description': new_repo.description,
                'webhook_url': new_repo.webhook_url,
                'filesystem_path': new_repo.filesystem_path,
                'ssh_password': new_repo.ssh_password,
                'is_private': new_repo.is_private,
                'default_branch': new_repo.default_branch,
                'created_at': new_repo.created_at.isoformat() if new_repo.created_at else None,
                'updated_at': new_repo.updated_at.isoformat() if new_repo.updated_at else None
            }
            
            session.close()
            logger.info(f"Created repository: {repo_dict['name']}")
            return jsonify(repo_dict), 201
        except Exception as e:
            logger.error(f"Error creating repository: {e}")
            return jsonify({'error': str(e)}), 500

    @repositories_bp.route('/api/repositories/<int:repo_id>', methods=['PUT'])
    def update_repository(repo_id):
        """Update a repository"""
        try:
            data = request.get_json()
            session = Session(db_engine)
            
            # Get repository
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)
            
            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404
            
            # Update fields
            if 'name' in data:
                repo.name = data['name']
            if 'owner' in data:
                repo.owner = data['owner']
            if 'url' in data:
                repo.url = data['url']
            if 'description' in data:
                repo.description = data['description']
            if 'webhook_url' in data:
                repo.webhook_url = data['webhook_url']
            if 'filesystem_path' in data:
                repo.filesystem_path = data['filesystem_path']
            if 'ssh_password' in data:
                repo.ssh_password = data['ssh_password']
            if 'is_private' in data:
                repo.is_private = data['is_private']
            if 'default_branch' in data:
                repo.default_branch = data['default_branch']
            
            repo.updated_at = datetime.utcnow()
            
            session.commit()
            session.refresh(repo)
            
            repo_dict = {
                'id': repo.id,
                'name': repo.name,
                'owner': repo.owner,
                'url': repo.url,
                'description': repo.description,
                'webhook_url': repo.webhook_url,
                'filesystem_path': repo.filesystem_path,
                'ssh_password': repo.ssh_password,
                'is_private': repo.is_private,
                'default_branch': repo.default_branch,
                'created_at': repo.created_at.isoformat() if repo.created_at else None,
                'updated_at': repo.updated_at.isoformat() if repo.updated_at else None
            }
            
            session.close()
            logger.info(f"Updated repository {repo_id}")
            return jsonify(repo_dict), 200
        except Exception as e:
            logger.error(f"Error updating repository {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500

    @repositories_bp.route('/api/repositories/<int:repo_id>', methods=['DELETE'])
    def delete_repository(repo_id):
        """Delete a repository"""
        try:
            session = Session(db_engine)
            
            # Get repository
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)
            
            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404
            
            # Delete repository
            session.delete(repo)
            session.commit()
            session.close()
            
            logger.info(f"Deleted repository {repo_id}")
            return jsonify({'message': 'Repository deleted successfully'}), 200
        except Exception as e:
            logger.error(f"Error deleting repository {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500

    @repositories_bp.route('/api/repositories/<int:repo_id>/filesystem-status', methods=['GET'])
    def get_repository_filesystem_status(repo_id):
        """Get filesystem status for a repository (git repo and docker compose presence)"""
        try:
            session = Session(db_engine)
            
            # Get repository
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)
            
            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404
            
            filesystem_path = repo.filesystem_path
            if not filesystem_path:
                session.close()
                return jsonify({
                    'has_git_repo': False,
                    'has_docker_compose': False
                }), 200
            
            has_git_repo = check_git_repo(filesystem_path)
            has_docker_compose = check_docker_compose(filesystem_path)
            
            session.close()
            logger.info(f"Checked filesystem status for repository {repo_id}: git={has_git_repo}, compose={has_docker_compose}")
            return jsonify({
                'has_git_repo': has_git_repo,
                'has_docker_compose': has_docker_compose
            }), 200
        except Exception as e:
            logger.error(f"Error checking filesystem status for repository {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500

    @repositories_bp.route('/api/repositories/<int:repo_id>/compose-files', methods=['GET'])
    def get_repository_compose_files(repo_id):
        """Return a list of docker-compose files with content and status (DockerComposeFile[])"""
        try:
            session = Session(db_engine)
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)

            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404

            filesystem_path = repo.filesystem_path
            if not filesystem_path or not os.path.exists(filesystem_path):
                session.close()
                return jsonify({'error': 'Filesystem path not found'}), 404

            possible_files = ['docker-compose.yml', 'docker-compose.yaml', 'compose.yml', 'compose.yaml']
            found_files = []

            for root, dirs, files in os.walk(filesystem_path):
                for candidate in possible_files:
                    if candidate in files:
                        full_path = os.path.join(root, candidate)
                        found_files.append(full_path)

            # Try to init docker client for status calculation
            docker_client = None
            try:
                docker_client = docker.from_env()
            except Exception as e:
                logger.warning(f"Docker client unavailable when listing compose files: {e}")

            result = []
            for full_path in found_files:
                try:
                    with open(full_path, 'r', encoding='utf-8') as fh:
                        content = fh.read()
                except Exception as e:
                    logger.error(f"Failed reading compose file {full_path}: {e}")
                    content = ''

                name = os.path.basename(full_path)
                # Heuristic project name: folder name containing the compose file
                project_name = os.path.basename(os.path.dirname(full_path))

                status = 'stopped'
                if docker_client:
                    try:
                        containers = docker_client.containers.list(
                            all=True,
                            filters={'label': f'com.docker.compose.project={project_name}'}
                        )
                        if any(c.status == 'running' for c in containers):
                            status = 'running'
                        elif containers:
                            status = 'stopped'
                        else:
                            status = 'stopped'
                    except Exception as e:
                        logger.warning(f"Error checking docker status for project {project_name}: {e}")
                        status = 'error'

                result.append({
                    'id': full_path,  # unique enough for UI; path as id
                    'name': name,
                    'path': full_path,
                    'status': status,
                    'content': content,
                })

            session.close()
            return jsonify(result), 200
        except Exception as e:
            logger.error(f"Error listing compose files for repository {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500

    

    @repositories_bp.route('/api/repositories/<int:repo_id>/git/status', methods=['GET'])
    def git_status(repo_id):
        try:
            session = Session(db_engine)
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)
            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404

            path = repo.filesystem_path
            if not path or not check_git_repo(path):
                session.close()
                return jsonify({'error': 'Not a git repository'}), 400

            status_res = run_git_cmd(path, ['status', '--porcelain', '-b'])
            # also get diff with zero context to identify exact changed line numbers
            diff_res = run_git_cmd(path, ['diff', '--unified=0'])

            parsed_diff = None
            if diff_res and diff_res.get('stdout'):
                try:
                    parsed_diff = parse_unified_diff(diff_res['stdout'])
                except Exception as e:
                    logger.error(f"Failed to parse diff for {repo_id}: {e}")

            session.close()
            out = {
                'status': status_res,
                'diff_raw': diff_res.get('stdout') if diff_res else '',
                'diff': parsed_diff,
            }
            return jsonify(out), 200
        except Exception as e:
            logger.error(f"Error running git status for {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500

    @repositories_bp.route('/api/repositories/<int:repo_id>/git/fetch', methods=['POST'])
    def git_fetch(repo_id):
        try:
            session = Session(db_engine)
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)
            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404

            path = repo.filesystem_path
            if not path or not check_git_repo(path):
                session.close()
                return jsonify({'error': 'Not a git repository'}), 400

            res = run_git_cmd(path, ['fetch', '--all'], ssh_password=repo.ssh_password)
            session.close()
            return jsonify(res), 200
        except Exception as e:
            logger.error(f"Error running git fetch for {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500

    @repositories_bp.route('/api/repositories/<int:repo_id>/git/pull', methods=['POST'])
    def git_pull(repo_id):
        try:
            data = request.get_json() or {}
            remote = data.get('remote', 'origin')
            branch = data.get('branch')

            session = Session(db_engine)
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)
            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404

            path = repo.filesystem_path
            if not path or not check_git_repo(path):
                session.close()
                return jsonify({'error': 'Not a git repository'}), 400

            # determine branch
            if not branch:
                branch = repo.default_branch or 'main'

            res = run_git_cmd(path, ['pull', remote, branch], ssh_password=repo.ssh_password)
            session.close()
            return jsonify(res), 200
        except Exception as e:
            logger.error(f"Error running git pull for {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500
        
    @repositories_bp.route('/api/repositories/<int:repo_id>/git/log', methods=['GET'])
    def git_log(repo_id):
        try:
            session = Session(db_engine)
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)
            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404

            path = repo.filesystem_path
            if not path or not check_git_repo(path):
                session.close()
                return jsonify({'error': 'Not a git repository'}), 400

            res = run_git_cmd(path, ['log', '--oneline', '-n', '20'])
            session.close()
            return jsonify(res), 200
        except Exception as e:
            logger.error(f"Error running git log for {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500
    
    @repositories_bp.route('/api/repositories/<int:repo_id>/git/config', methods=['GET'])
    def git_config(repo_id):
        """Get git config for the repository"""
        try:
            session = Session(db_engine)
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)
            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404

            path = repo.filesystem_path
            if not path or not check_git_repo(path):
                session.close()
                return jsonify({'error': 'Not a git repository'}), 400

            # Get git config list
            res = run_git_cmd(path, ['config', '--list', '--local'])
            
            config = {}
            if res['returncode'] == 0 and res['stdout']:
                # Parse config output
                for line in res['stdout'].strip().split('\n'):
                    if '=' in line:
                        key, value = line.split('=', 1)
                        config[key] = value
            
            session.close()
            return jsonify({
                'returncode': res['returncode'],
                'config': config,
                'stderr': res['stderr']
            }), 200
        except Exception as e:
            logger.error(f"Error getting git config for {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500
    
    @repositories_bp.route('/api/repositories/<int:repo_id>/git/stash', methods=['POST'])
    def git_stash(repo_id):
        """Run git stash on the repository"""
        try:
            data = request.get_json() or {}
            message = data.get('message', '')
            
            session = Session(db_engine)
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)
            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404

            path = repo.filesystem_path
            if not path or not check_git_repo(path):
                session.close()
                return jsonify({'error': 'Not a git repository'}), 400

            # Run git stash
            if message:
                res = run_git_cmd(path, ['stash', 'push', '-m', message])
            else:
                res = run_git_cmd(path, ['stash'])
            
            session.close()
            return jsonify(res), 200
        except Exception as e:
            logger.error(f"Error running git stash for {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500
        
    @repositories_bp.route('/api/repositories/<int:repo_id>/docker-compose', methods=['GET'])
    def repository_scan_compose_files(repo_id):
        """Scan all docker-compose files in the repository's filesystem path"""
        try:
            session = Session(db_engine)
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)

            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404
            
            filesystem_path = repo.filesystem_path
            if not filesystem_path or not os.path.exists(filesystem_path):
                session.close()
                return jsonify({'error': 'Filesystem path not found'}), 404

            compose_files = []
            possible_files = ['docker-compose.yml', 'docker-compose.yaml', 'compose.yml', 'compose.yaml']
            for root, dirs, files in os.walk(filesystem_path):
                for f in possible_files:
                    if f in files:
                        full_path = os.path.join(root, f)
                        compose_files.append(full_path)

            session.close()
            return jsonify({'compose_files': compose_files}), 200
        except Exception as e:
            logger.error(f"Error scanning compose files for repository {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500
        
    @repositories_bp.route('/api/repositories/<int:repo_id>/readme', methods=['POST'])
    def repository_readme(repo_id):
        """Get the README.md content from the repository's filesystem path"""
        try:
            session = Session(db_engine)
            stmt = select(Repository).where(Repository.id == repo_id)
            repo = session.scalar(stmt)

            if not repo:
                session.close()
                return jsonify({'error': 'Repository not found'}), 404
            
            data = request.get_json() or {}
            path = data.get('filesystem_path')

            filesystem_path = path or repo.filesystem_path
            if not filesystem_path or not os.path.exists(filesystem_path):
                session.close()
                return jsonify({'error': 'Filesystem path not found'}), 404

            readme_path = os.path.join(filesystem_path, 'README.md')
            if not os.path.isfile(readme_path):
                session.close()
                return jsonify({'error': 'README.md not found'}), 404

            with open(readme_path, 'r', encoding='utf-8') as f:
                content = f.read()

            session.close()
            return jsonify({'content': content}), 200
        except Exception as e:
            logger.error(f"Error reading README for repository {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500
    
    app.register_blueprint(repositories_bp)
