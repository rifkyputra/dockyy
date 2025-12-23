import os
import subprocess
import re
import tempfile
from typing import List, Dict, Any, Optional


def check_git_repo(path: str) -> bool:
    """Check if the given path contains a git repository"""
    if not path or not os.path.exists(path):
        return False

    try:
        git_dir = os.path.join(path, '.git')
        if os.path.isdir(git_dir):
            return True

        result = subprocess.run(
            ['git', 'rev-parse', '--git-dir'],
            cwd=path,
            capture_output=True,
            text=True,
            timeout=5,
        )
        return result.returncode == 0
    except (subprocess.TimeoutExpired, subprocess.SubprocessError, OSError):
        return False


def run_git_cmd(path: str, args: List[str], timeout: int = 30, ssh_password: Optional[str] = None) -> Dict[str, Any]:
    """
    Run a git command with optional SSH password authentication.
    
    Args:
        path: The repository path
        args: Git command arguments
        timeout: Command timeout in seconds
        ssh_password: Optional SSH password for authentication
    
    Returns:
        Dictionary with returncode, stdout, and stderr
    """
    try:
        env = os.environ.copy()
        askpass_script = None
        
        # Check for local git config core.sshcommand
        git_ssh_command = None
        try:
            result = subprocess.run(
                ['git', 'config', '--local', 'core.sshcommand'],
                cwd=path,
                capture_output=True,
                text=True,
                timeout=5,
            )
            if result.returncode == 0 and result.stdout.strip():
                git_ssh_command = result.stdout.strip()
        except Exception:
            pass
        
        # If SSH password is provided, create a temporary askpass script
        if ssh_password:
            # Create a temporary script that returns the password
            askpass_script = tempfile.NamedTemporaryFile(mode='w', delete=False, suffix='.sh')
            askpass_script.write('#!/bin/sh\n')
            askpass_script.write(f'echo "{ssh_password}"\n')
            askpass_script.close()
            
            # Make the script executable
            os.chmod(askpass_script.name, 0o700)
            
            # Set environment variables for SSH password authentication
            env['SSH_ASKPASS'] = askpass_script.name
            env['SSH_ASKPASS_REQUIRE'] = 'force'
            env['DISPLAY'] = ':0'  # Required for SSH_ASKPASS to work
            
            # If there's a local git config, append our options to it
            if git_ssh_command:
                env['GIT_SSH_COMMAND'] = f'{git_ssh_command} -o StrictHostKeyChecking=no'
            else:
                env['GIT_SSH_COMMAND'] = 'ssh -o StrictHostKeyChecking=no'
        elif git_ssh_command:
            # Use the local git config core.sshcommand
            env['GIT_SSH_COMMAND'] = git_ssh_command
        
        result = subprocess.run(
            ['git', *args],
            cwd=path,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
        )
        
        # Clean up the temporary askpass script
        if askpass_script and os.path.exists(askpass_script.name):
            os.unlink(askpass_script.name)
        
        return {
            'returncode': result.returncode,
            'stdout': result.stdout,
            'stderr': result.stderr,
        }
    except subprocess.TimeoutExpired as e:
        if askpass_script and os.path.exists(askpass_script.name):
            os.unlink(askpass_script.name)
        return {'returncode': 124, 'stdout': '', 'stderr': f'Timeout: {e}'}
    except Exception as e:
        if askpass_script and os.path.exists(askpass_script.name):
            os.unlink(askpass_script.name)
        return {'returncode': 1, 'stdout': '', 'stderr': str(e)}


def parse_unified_diff(diff_text: str) -> List[Dict[str, Any]]:
    """Parse unified diff output into structured JSON with hunks and line changes."""
    files = []
    cur = None
    hunk_re = re.compile(r"@@ -([0-9]+)(?:,([0-9]+))? \+([0-9]+)(?:,([0-9]+))? @@")

    lines = diff_text.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith('diff --git'):
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

            i += 1
            old_line = old_start
            new_line = new_start
            while i < len(lines):
                l = lines[i]
                if l.startswith('diff --git') or l.startswith('@@ '):
                    break
                if l.startswith('+') and not l.startswith('+++'):
                    hunk['changes'].append({'type': 'add', 'line': new_line, 'content': l[1:]})
                    new_line += 1
                elif l.startswith('-') and not l.startswith('---'):
                    hunk['changes'].append({'type': 'del', 'line': old_line, 'content': l[1:]})
                    old_line += 1
                else:
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
