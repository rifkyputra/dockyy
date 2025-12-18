import os
from typing import Any


def check_docker_compose(path: str) -> bool:
    """Check if the given path contains docker-compose files"""
    if not path or not os.path.exists(path):
        return False

    compose_files = ['docker-compose.yml', 'docker-compose.yaml', 'compose.yml', 'compose.yaml']
    return any(os.path.isfile(os.path.join(path, file)) for file in compose_files)
