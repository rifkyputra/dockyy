from flask import Blueprint, jsonify, request
from sqlalchemy import select
from sqlalchemy.orm import Session
from app.models import Repository
from datetime import datetime
import logging
import os
import subprocess

logger = logging.getLogger(__name__)

def check_git_repo(path: str) -> bool:
    """Check if the given path contains a git repository"""
    if not path or not os.path.exists(path):
        return False
    
    try:
        # Check if .git directory exists
        git_dir = os.path.join(path, '.git')
        if os.path.isdir(git_dir):
            return True
        
        # Also check if git command recognizes it as a repo
        result = subprocess.run(
            ['git', 'rev-parse', '--git-dir'],
            cwd=path,
            capture_output=True,
            text=True,
            timeout=5
        )
        return result.returncode == 0
    except (subprocess.TimeoutExpired, subprocess.SubprocessError, OSError):
        return False

def check_docker_compose(path: str) -> bool:
    """Check if the given path contains docker-compose files"""
    if not path or not os.path.exists(path):
        return False
    
    compose_files = ['docker-compose.yml', 'docker-compose.yaml', 'compose.yml', 'compose.yaml']
    return any(os.path.isfile(os.path.join(path, file)) for file in compose_files)

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

    @repositories_bp.route('/api/repositories/<int:repo_id>/compose-file', methods=['GET'])
    def get_repository_compose_file(repo_id):
        """Return docker-compose file content for a repository if present"""
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

            # Look for common compose filenames
            compose_files = ['docker-compose.yml', 'docker-compose.yaml', 'compose.yml', 'compose.yaml']
            found = None
            for f in compose_files:
                candidate = os.path.join(filesystem_path, f)
                if os.path.isfile(candidate):
                    found = candidate
                    break

            if not found:
                session.close()
                return jsonify({'error': 'No compose file found'}), 404

            with open(found, 'r', encoding='utf-8') as fh:
                content = fh.read()

            session.close()
            return jsonify({'path': found, 'content': content}), 200
        except Exception as e:
            logger.error(f"Error reading compose file for repository {repo_id}: {e}")
            return jsonify({'error': str(e)}), 500

    def run_git_cmd(path: str, args: list, timeout: int = 30):
        try:
            result = subprocess.run(
                ['git', *args],
                cwd=path,
                capture_output=True,
                text=True,
                timeout=timeout,
            )
            return {
                'returncode': result.returncode,
                'stdout': result.stdout,
                'stderr': result.stderr,
            }
        except subprocess.TimeoutExpired as e:
            return {'returncode': 124, 'stdout': '', 'stderr': f'Timeout: {e}'}
        except Exception as e:
            return {'returncode': 1, 'stdout': '', 'stderr': str(e)}

    def parse_unified_diff(diff_text: str):
        """Parse unified diff output into structured JSON with hunks and line changes."""
        import re

        files = []
        cur = None
        hunk_re = re.compile(r"@@ -([0-9]+)(?:,([0-9]+))? \+([0-9]+)(?:,([0-9]+))? @@")

        lines = diff_text.splitlines()
        i = 0
        while i < len(lines):
            line = lines[i]
            if line.startswith('diff --git'):
                # start new file
                if cur:
                    files.append(cur)
                cur = {'from': None, 'to': None, 'hunks': []}
                i += 1
                continue

            if cur is not None and line.startswith('--- '):
                cur['from'] = line[4:].strip()
                i += 1
                continue

            if cur is not None and line.startswith('+++ '):
                cur['to'] = line[4:].strip()
                i += 1
                continue

            m = hunk_re.match(line)
            if cur is not None and m:
                old_start = int(m.group(1))
                old_count = int(m.group(2)) if m.group(2) else 1
                new_start = int(m.group(3))
                new_count = int(m.group(4)) if m.group(4) else 1

                hunk = {
                    'old_start': old_start,
                    'old_count': old_count,
                    'new_start': new_start,
                    'new_count': new_count,
                    'changes': [],
                }

                # consume hunk lines
                i += 1
                old_line = old_start
                new_line = new_start
                while i < len(lines):
                    l = lines[i]
                    if l.startswith('diff --git') or l.startswith('@@ '):
                        # do not consume this line here
                        break
                    if l.startswith('+') and not l.startswith('+++'):
                        hunk['changes'].append({'type': 'add', 'line': new_line, 'content': l[1:]})
                        new_line += 1
                    elif l.startswith('-') and not l.startswith('---'):
                        hunk['changes'].append({'type': 'del', 'line': old_line, 'content': l[1:]})
                        old_line += 1
                    else:
                        # context line
                        if l.startswith(' '):
                            old_line += 1
                            new_line += 1
                    i += 1

                cur['hunks'].append(hunk)
                continue

            i += 1

        if cur:
            files.append(cur)

        return files

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

            res = run_git_cmd(path, ['fetch', '--all'])
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

            res = run_git_cmd(path, ['pull', remote, branch])
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
