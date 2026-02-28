use anyhow::Result;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tokio::process::Command;
use crate::AppState;
use crate::db::models::{Job, Repository};
use crate::services::traefik::{TraefikService, TRAEFIK_NETWORK};
use serde_json::Value;

pub async fn run_worker(state: Arc<AppState>) {
    tracing::info!("Starting background worker loop");
    
    loop {
        if let Err(e) = process_next_job(&state).await {
            // Only log errors if they are not "no jobs found"
            // Wait, we'll design process_next_job to return Ok(bool) where bool is "did we find a job"
            match e.to_string().as_str() {
                "No pending jobs" => {},
                _ => tracing::error!("Worker error: {}", e),
            }
        }
        
        sleep(Duration::from_secs(5)).await;
    }
}

async fn process_next_job(state: &Arc<AppState>) -> Result<()> {
    // 1. Find a pending job
    let job = state.db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, job_type, payload, status, result, attempts, max_attempts, created_at, updated_at 
             FROM jobs WHERE status = 'pending' ORDER BY created_at ASC LIMIT 1"
        )?;
        
        let job = stmt.query_row([], |row| {
            Ok(Job {
                id: row.get(0)?,
                job_type: row.get(1)?,
                payload: row.get(2)?,
                status: row.get(3)?,
                result: row.get(4)?,
                attempts: row.get(5)?,
                max_attempts: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        }).map_err(|_| anyhow::anyhow!("No pending jobs"))?;
        
        Ok(job)
    })?;

    tracing::info!("Processing job {} (type: {})", job.id, job.job_type);

    // 2. Mark job as running
    state.db.with_conn(|conn| {
        conn.execute("UPDATE jobs SET status = 'running', updated_at = datetime('now') WHERE id = ?1", [job.id])?;
        Ok(())
    })?;

    // 3. Dispatch based on job type
    let result = match job.job_type.as_str() {
        "deploy" => handle_deploy_job(state, &job).await,
        _ => Err(anyhow::anyhow!("Unknown job type: {}", job.job_type)),
    };

    // 4. Update job status based on result
    match result {
        Ok(_) => {
            tracing::info!("Job {} completed successfully", job.id);
            state.db.with_conn(|conn| {
                conn.execute(
                    "UPDATE jobs SET status = 'completed', updated_at = datetime('now') WHERE id = ?1",
                    [job.id]
                )?;
                Ok(())
            })?;
        }
        Err(e) => {
            tracing::error!("Job {} failed: {}", job.id, e);
            state.db.with_conn(|conn| {
                conn.execute(
                    "UPDATE jobs SET status = 'failed', result = ?2, attempts = attempts + 1, updated_at = datetime('now') WHERE id = ?1",
                    rusqlite::params![job.id, e.to_string()]
                )?;
                Ok(())
            })?;
        }
    }

    Ok(())
}

async fn handle_deploy_job(state: &Arc<AppState>, job: &Job) -> Result<()> {
    let payload: Value = serde_json::from_str(&job.payload)?;
    let repo_id = payload["repo_id"].as_i64().ok_or_else(|| anyhow::anyhow!("Missing repo_id in payload"))?;
    
    // 1. Get repository info
    let repo = state.db.with_conn(|conn| {
        let repo = conn.query_row(
            "SELECT id, name, owner, url, description, webhook_url, filesystem_path,
                    ssh_password, is_private, default_branch, domain, proxy_port,
                    created_at, updated_at
             FROM repositories WHERE id = ?1",
            [repo_id],
            |row| {
                Ok(Repository {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    owner: row.get(2)?,
                    url: row.get(3)?,
                    description: row.get(4)?,
                    webhook_url: row.get(5)?,
                    filesystem_path: row.get(6)?,
                    ssh_password: row.get(7)?,
                    is_private: row.get::<_, i64>(8)? != 0,
                    default_branch: row.get(9)?,
                    domain: row.get(10)?,
                    proxy_port: row.get(11)?,
                    created_at: row.get(12)?,
                    updated_at: row.get(13)?,
                })
            }
        )?;
        Ok(repo)
    })?;

    // 2. Create deployment record
    let deployment_id = state.db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO deployments (repo_id, status) VALUES (?1, 'building')",
            [repo.id]
        )?;
        Ok(conn.last_insert_rowid())
    })?;

    let repo_dir = format!("{}/repos/{}", state.config.data_dir, repo.id);
    std::fs::create_dir_all(&repo_dir)?;

    let mut temp_key_path = None;
    let git_ssh_command = if let Some(ssh_key) = &repo.ssh_password {
        if !ssh_key.trim().is_empty() {
            let key_path = format!("{}/repos/{}_id_rsa", state.config.data_dir, repo.id);
            std::fs::write(&key_path, ssh_key.trim())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&key_path)?.permissions();
                perms.set_mode(0o600);
                std::fs::set_permissions(&key_path, perms)?;
            }
            temp_key_path = Some(key_path.clone());
            format!("ssh -i {} -o StrictHostKeyChecking=no", key_path)
        } else {
            "ssh -o StrictHostKeyChecking=no".to_string()
        }
    } else {
        "ssh -o StrictHostKeyChecking=no".to_string()
    };

    // 3. Clone or Pull
    if std::path::Path::new(&format!("{}/.git", repo_dir)).exists() {
        tracing::info!("Pulling repo {}", repo.name);
        let output = Command::new("git")
            .env("GIT_SSH_COMMAND", &git_ssh_command)
            .arg("-C")
            .arg(&repo_dir)
            .arg("pull")
            .arg("origin")
            .arg(&repo.default_branch)
            .output().await?;
            
        if let Some(key_path) = &temp_key_path {
            let _ = std::fs::remove_file(key_path);
        }
        
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Git pull failed: {}", err));
        }
    } else {
        tracing::info!("Cloning repo {} to {}", repo.url, repo_dir);
        let output = Command::new("git")
            .env("GIT_SSH_COMMAND", &git_ssh_command)
            .arg("clone")
            .arg(&repo.url)
            .arg(&repo_dir)
            .output().await?;
            
        if let Some(key_path) = &temp_key_path {
            let _ = std::fs::remove_file(key_path);
        }
            
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Git clone failed: {}", err));
        }
    }

    // 4. Build with Nixpacks (or just build as a Docker image)
    // We'll tag the image as dockyy-{repo_name}:latest
    let image_tag = format!("dockyy-{}:latest", repo.name.to_lowercase().replace("/", "-"));
    tracing::info!("Building image {}", image_tag);
    
    // We use nixpacks if available, otherwise we assume a Dockerfile exists
    // Let's try nixpacks first
    let build_output = Command::new("nixpacks")
        .arg("build")
        .arg(&repo_dir)
        .arg("--name")
        .arg(&image_tag)
        .output().await;

    let build_log = match build_output {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).to_string()
        },
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("Nixpacks build failed: {}. Trying docker build...", err);
            // Fallback to docker build .
            let dbuild = Command::new("docker")
                .arg("build")
                .arg("-t")
                .arg(&image_tag)
                .arg(&repo_dir)
                .output().await?;
            if !dbuild.status.success() {
                return Err(anyhow::anyhow!("Docker build failed: {}", String::from_utf8_lossy(&dbuild.stderr)));
            }
            String::from_utf8_lossy(&dbuild.stdout).to_string()
        },
        Err(e) => {
            tracing::warn!("Nixpacks not found: {}. Trying docker build...", e);
            let dbuild = Command::new("docker")
                .arg("build")
                .arg("-t")
                .arg(&image_tag)
                .arg(&repo_dir)
                .output().await?;
             if !dbuild.status.success() {
                return Err(anyhow::anyhow!("Docker build failed: {}", String::from_utf8_lossy(&dbuild.stderr)));
            }
            String::from_utf8_lossy(&dbuild.stdout).to_string()
        }
    };

    // 5. Deploy / Start container
    let container_name = format!("dockyy-{}", repo.name.to_lowercase().replace("/", "-"));
    let _ = state.docker.stop_container(&container_name).await;
    let _ = state.docker.remove_container(&container_name, true).await;

    // Ensure the shared proxy network exists before running the container
    state.traefik.ensure_network().await?;

    tracing::info!("Starting container {}", container_name);

    let mut run_cmd = Command::new("docker");
    run_cmd
        .arg("run")
        .arg("-d")
        .arg("--name")
        .arg(&container_name)
        .arg("--network")
        .arg(TRAEFIK_NETWORK)
        .arg("--restart")
        .arg("always");

    // Attach Traefik routing labels when a domain is configured
    if let Some(ref domain) = repo.domain {
        let proxy_port = repo.proxy_port.unwrap_or(3000) as u16;
        tracing::info!(
            "Attaching Traefik route: {} -> {}:{}",
            domain,
            container_name,
            proxy_port
        );
        let labels = TraefikService::container_labels(&container_name, domain, proxy_port);
        for (k, v) in &labels {
            run_cmd.arg("--label").arg(format!("{}={}", k, v));
        }
    }

    run_cmd.arg(&image_tag);

    let run_output = run_cmd.output().await?;

    if !run_output.status.success() {
        return Err(anyhow::anyhow!(
            "Docker run failed: {}",
            String::from_utf8_lossy(&run_output.stderr)
        ));
    }

    let container_id = String::from_utf8_lossy(&run_output.stdout)
        .trim()
        .to_string();

    // 6. Update deployment record (persist domain for reference)
    let domain_val = repo.domain.clone();
    let port_val = repo.proxy_port;
    state.db.with_conn(|conn| {
        conn.execute(
            "UPDATE deployments
             SET status = 'success', container_id = ?2, image_name = ?3,
                 build_log = ?4, domain = ?5, port = ?6,
                 updated_at = datetime('now')
             WHERE id = ?1",
            rusqlite::params![deployment_id, container_id, image_tag, build_log, domain_val, port_val]
        )?;
        Ok(())
    })?;

    Ok(())
}
