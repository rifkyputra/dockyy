use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{get, post, put},
    Json, Router,
};
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::db::models::{
    CreateRepository, DockerComposeUpRequest, Repository, SaveComposeOverrideRequest,
    UpdateRepository,
};
use crate::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/repositories", get(list_repositories).post(create_repository))
        .route(
            "/repositories/{id}",
            get(get_repository)
                .put(update_repository)
                .delete(delete_repository),
        )
        .route("/repositories/{id}/filesystem-status", get(get_filesystem_status))
        .route("/repositories/{id}/readme", get(get_readme))
        .route("/repositories/{id}/compose-files", get(get_compose_files))
        .route("/repositories/{id}/clone", post(clone_repository))
        .route("/repositories/{id}/pull", post(pull_repository))
        .route("/repositories/{id}/fetch", post(fetch_repository))
        .route("/repositories/{id}/docker-compose-up", post(docker_compose_up))
        .route("/repositories/{id}/docker-compose-up/stream", get(docker_compose_up_stream))
        .route(
            "/repositories/{id}/compose-files/{filename}",
            put(save_compose_override).delete(reset_compose_override),
        )
}

async fn list_repositories(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Repository>>, (StatusCode, Json<Value>)> {
    state
        .db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, owner, url, description, webhook_url, filesystem_path,
                        ssh_password, is_private, default_branch, domain, proxy_port,
                        created_at, updated_at
                 FROM repositories ORDER BY updated_at DESC",
            )?;

            let repos = stmt
                .query_map([], |row| {
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
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(repos)
        })
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })
}

async fn get_repository(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Repository>, (StatusCode, Json<Value>)> {
    state
        .db
        .with_conn(|conn| {
            let repo = conn.query_row(
                "SELECT id, name, owner, url, description, webhook_url, filesystem_path,
                        ssh_password, is_private, default_branch, domain, proxy_port,
                        created_at, updated_at
                 FROM repositories WHERE id = ?1",
                [id],
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
                },
            )?;
            Ok(repo)
        })
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": e.to_string()})),
            )
        })
}

async fn create_repository(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateRepository>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO repositories (name, owner, url, description, webhook_url,
                    filesystem_path, ssh_password, is_private, default_branch,
                    domain, proxy_port)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    body.name,
                    body.owner,
                    body.url,
                    body.description,
                    body.webhook_url,
                    body.filesystem_path,
                    body.ssh_password,
                    body.is_private as i64,
                    body.default_branch,
                    body.domain,
                    body.proxy_port,
                ],
            )?;
            let id = conn.last_insert_rowid();
            Ok(id)
        })
        .map(|id| {
            (
                StatusCode::CREATED,
                Json(json!({"id": id, "message": "Repository created"})),
            )
        })
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })
}

async fn update_repository(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateRepository>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state
        .db
        .with_conn(|conn| {
            // Build dynamic SET clause
            let mut sets = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref name) = body.name {
                sets.push("name = ?");
                params.push(Box::new(name.clone()));
            }
            if let Some(ref owner) = body.owner {
                sets.push("owner = ?");
                params.push(Box::new(owner.clone()));
            }
            if let Some(ref url) = body.url {
                sets.push("url = ?");
                params.push(Box::new(url.clone()));
            }
            if let Some(ref desc) = body.description {
                sets.push("description = ?");
                params.push(Box::new(desc.clone()));
            }
            if let Some(ref wh) = body.webhook_url {
                sets.push("webhook_url = ?");
                params.push(Box::new(wh.clone()));
            }
            if let Some(ref fp) = body.filesystem_path {
                sets.push("filesystem_path = ?");
                params.push(Box::new(fp.clone()));
            }
            if let Some(ref sp) = body.ssh_password {
                sets.push("ssh_password = ?");
                params.push(Box::new(sp.clone()));
            }
            if let Some(is_priv) = body.is_private {
                sets.push("is_private = ?");
                params.push(Box::new(is_priv as i64));
            }
            if let Some(ref branch) = body.default_branch {
                sets.push("default_branch = ?");
                params.push(Box::new(branch.clone()));
            }
            if let Some(ref d) = body.domain {
                sets.push("domain = ?");
                params.push(Box::new(d.clone()));
            }
            if let Some(pp) = body.proxy_port {
                sets.push("proxy_port = ?");
                params.push(Box::new(pp));
            }

            if sets.is_empty() {
                anyhow::bail!("No fields to update");
            }

            sets.push("updated_at = datetime('now')");
            params.push(Box::new(id));

            let sql = format!(
                "UPDATE repositories SET {} WHERE id = ?",
                sets.join(", ")
            );

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            conn.execute(&sql, param_refs.as_slice())?;

            Ok(())
        })
        .map(|_| Json(json!({"message": "Repository updated"})))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })
}

async fn delete_repository(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state
        .db
        .with_conn(|conn| {
            let changes = conn.execute("DELETE FROM repositories WHERE id = ?1", [id])?;
            if changes == 0 {
                anyhow::bail!("Repository not found");
            }
            Ok(())
        })
        .map(|_| Json(json!({"message": "Repository deleted"})))
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": e.to_string()})),
            )
        })
}

async fn get_filesystem_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let repo_dir = format!("{}/repos/{}", state.config.data_dir, id);
    let path = std::path::Path::new(&repo_dir);
    
    let has_git_repo = path.join(".git").exists();
    let has_docker_compose = std::fs::read_dir(path).map(|mut entries| {
        entries.any(|entry| {
            if let Ok(entry) = entry {
                if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    return false;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                (name.starts_with("docker-compose") || name.starts_with("compose"))
                    && (name.ends_with(".yml") || name.ends_with(".yaml"))
            } else {
                false
            }
        })
    }).unwrap_or(false);
    let absolute_path = match std::fs::canonicalize(path) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => {
            let pwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            pwd.join(path).to_string_lossy().to_string()
        }
    };
    
    Ok(Json(json!({
        "has_git_repo": has_git_repo,
        "has_docker_compose": has_docker_compose,
        "repo_path": absolute_path
    })))
}

async fn get_readme(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let repo_dir = format!("{}/repos/{}", state.config.data_dir, id);
    let path = std::path::Path::new(&repo_dir);
    
    let readme_paths = ["README.md", "readme.md", "Readme.md", "README.MD"];
    let mut content = String::new();
    
    for rp in readme_paths.iter() {
        if let Ok(c) = std::fs::read_to_string(path.join(rp)) {
            content = c;
            break;
        }
    }
    
    Ok(Json(json!({
        "content": content
    })))
}

fn override_dir(data_dir: &str, repo_id: i64) -> String {
    format!("{}/compose-overrides/{}", data_dir, repo_id)
}

async fn get_compose_files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let repo_dir = format!("{}/repos/{}", state.config.data_dir, id);
    let path = std::path::Path::new(&repo_dir);
    let ovr_dir = override_dir(&state.config.data_dir, id);

    let mut files = Vec::new();

    if let Ok(entries) = std::fs::read_dir(path) {
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    (name.starts_with("docker-compose") || name.starts_with("compose"))
                        && (name.ends_with(".yml") || name.ends_with(".yaml"))
                } else {
                    false
                }
            })
            .collect();

        paths.sort();

        for cp in paths {
            if let Ok(c) = std::fs::read_to_string(&cp) {
                if let Some(name) = cp.file_name().and_then(|n| n.to_str()) {
                    let override_content =
                        std::fs::read_to_string(format!("{}/{}", ovr_dir, name)).ok();
                    files.push(json!({
                        "path": name,
                        "content": c,
                        "override_content": override_content
                    }));
                }
            }
        }
    }

    Ok(Json(json!(files)))
}

async fn clone_repository(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let repo = get_repository(State(state.clone()), Path(id)).await?.0;
    
    let repo_dir = format!("{}/repos/{}", state.config.data_dir, id);
    let _ = std::fs::remove_dir_all(&repo_dir);
    std::fs::create_dir_all(&repo_dir).unwrap();
    
    let mut cmd = tokio::process::Command::new(&state.config.git_bin);

    let mut temp_key_path = None;
    if let Some(ssh_key) = &repo.ssh_password {
        if !ssh_key.trim().is_empty() {
            let key_path = format!("{}/repos/{}_id_rsa", state.config.data_dir, id);
            std::fs::write(&key_path, ssh_key.trim()).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&key_path).unwrap().permissions();
                perms.set_mode(0o600);
                std::fs::set_permissions(&key_path, perms).unwrap();
            }
            cmd.env("GIT_SSH_COMMAND", format!("ssh -i {} -o StrictHostKeyChecking=no", key_path));
            temp_key_path = Some(key_path);
        } else {
            cmd.env("GIT_SSH_COMMAND", "ssh -o StrictHostKeyChecking=no");
        }
    } else {
        cmd.env("GIT_SSH_COMMAND", "ssh -o StrictHostKeyChecking=no");
    }

    tracing::info!(
        git_bin = %state.config.git_bin,
        repo_url = %repo.url,
        repo_dir = %repo_dir,
        "Cloning repository"
    );

    let output = cmd
        .arg("clone")
        .arg(&repo.url)
        .arg(&repo_dir)
        .output()
        .await
        .map_err(|e| {
            tracing::error!(
                git_bin = %state.config.git_bin,
                repo_url = %repo.url,
                error = %e,
                error_kind = ?e.kind(),
                "Failed to spawn git process"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("Failed to spawn git: {} (kind: {:?}, bin: {})", e, e.kind(), state.config.git_bin)})))
        })?;
        
    if let Some(key_path) = temp_key_path {
        let _ = std::fs::remove_file(key_path);
    }
        
    if !output.status.success() {
        
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": String::from_utf8_lossy(&output.stderr).to_string()}))));
    }
    
    Ok(Json(json!({"message": "Repository cloned successfully"})))
}

fn find_compose_bin() -> &'static str {
    if std::path::Path::new("/usr/bin/podman-compose").exists() {
        "/usr/bin/podman-compose"
    } else if std::path::Path::new("/usr/local/bin/podman-compose").exists() {
        "/usr/local/bin/podman-compose"
    } else if std::path::Path::new("/usr/bin/docker-compose").exists() {
        "/usr/bin/docker-compose"
    } else if std::path::Path::new("/usr/local/bin/docker-compose").exists() {
        "/usr/local/bin/docker-compose"
    } else {
        "docker-compose"
    }
}

fn setup_compose_cmd(
    compose_bin: &str,
    container_name: &str,
    repo_dir: &str,
    ovr_dir: &str,
    compose_file: Option<&str>,
) -> Result<(tokio::process::Command, Option<String>), (StatusCode, Json<Value>)> {
    let mut cmd = tokio::process::Command::new(compose_bin);
    cmd.arg("-p").arg(container_name);

    let mut temp_override_path: Option<String> = None;

    if let Some(file) = compose_file {
        if file.contains('/') || file.contains('\\') {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid compose file name"})),
            ));
        }
        let override_path = format!("{}/{}", ovr_dir, file);
        if std::path::Path::new(&override_path).exists() {
            let tmp_name = format!(".dockyy-override-{}", file);
            let tmp_path = format!("{}/{}", repo_dir, tmp_name);
            let content = std::fs::read_to_string(&override_path).map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
            })?;
            std::fs::write(&tmp_path, &content).map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
            })?;
            cmd.arg("-f").arg(&tmp_name);
            temp_override_path = Some(tmp_path);
        } else {
            cmd.arg("-f").arg(file);
        }
    }

    cmd.arg("up")
        .arg("-d")
        .arg("--build")
        .current_dir(repo_dir)
        .stdin(Stdio::null());

    Ok((cmd, temp_override_path))
}

async fn docker_compose_up(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<DockerComposeUpRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let repo = get_repository(State(state.clone()), Path(id)).await?.0;
    let repo_dir = format!("{}/repos/{}", state.config.data_dir, id);
    let ovr_dir = override_dir(&state.config.data_dir, id);
    let container_name = format!("dockyy-{}", repo.name.to_lowercase().replace("/", "-"));

    let compose_bin = find_compose_bin();
    let (mut cmd, temp_override_path) = setup_compose_cmd(
        compose_bin, &container_name, &repo_dir, &ovr_dir, body.compose_file.as_deref(),
    )?;

    // Create deployment record
    let deployment_id = state.db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO deployments (repo_id, status, domain, port) VALUES (?1, 'building', ?2, ?3)",
            rusqlite::params![id, repo.domain, repo.proxy_port],
        )?;
        Ok(conn.last_insert_rowid())
    }).map_err(|e: anyhow::Error| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
    })?;

    let output = cmd.output().await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
    })?;

    if let Some(tmp) = temp_override_path {
        let _ = std::fs::remove_file(tmp);
    }

    let build_log = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    if !output.status.success() {
        let _ = state.db.with_conn(|conn| {
            conn.execute(
                "UPDATE deployments SET status = 'failed', build_log = ?2, updated_at = datetime('now') WHERE id = ?1",
                rusqlite::params![deployment_id, build_log],
            )?;
            Ok(())
        });
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": String::from_utf8_lossy(&output.stderr).to_string()})),
        ));
    }

    let _ = state.db.with_conn(|conn| {
        conn.execute(
            "UPDATE deployments SET status = 'success', build_log = ?2, updated_at = datetime('now') WHERE id = ?1",
            rusqlite::params![deployment_id, build_log],
        )?;
        Ok(())
    });

    Ok(Json(json!({"message": "Deployment started with docker-compose", "deployment_id": deployment_id})))
}

#[derive(Deserialize)]
struct ComposeStreamQuery {
    compose_file: Option<String>,
}

async fn docker_compose_up_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(query): Query<ComposeStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, Json<Value>)> {
    let repo = get_repository(State(state.clone()), Path(id)).await?.0;
    let repo_dir = format!("{}/repos/{}", state.config.data_dir, id);
    let ovr_dir = override_dir(&state.config.data_dir, id);
    let container_name = format!("dockyy-{}", repo.name.to_lowercase().replace("/", "-"));

    let compose_bin = find_compose_bin();
    let (mut cmd, temp_override_path) = setup_compose_cmd(
        compose_bin, &container_name, &repo_dir, &ovr_dir, query.compose_file.as_deref(),
    )?;

    // Create deployment record
    let domain = repo.domain.clone();
    let proxy_port = repo.proxy_port;
    let deployment_id = state.db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO deployments (repo_id, status, domain, port) VALUES (?1, 'building', ?2, ?3)",
            rusqlite::params![id, domain, proxy_port],
        )?;
        Ok(conn.last_insert_rowid())
    }).map_err(|e: anyhow::Error| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
    })?;

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
    })?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Merge stdout and stderr into a single channel
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);

    if let Some(out) = stdout {
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx2.send(line).await;
            }
        });
    }

    if let Some(err_out) = stderr {
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(err_out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx2.send(line).await;
            }
        });
    }

    drop(tx);

    let stream = async_stream::stream! {
        let mut build_log = String::new();

        while let Some(line) = rx.recv().await {
            build_log.push_str(&line);
            build_log.push('\n');
            yield Ok::<_, Infallible>(Event::default().data(line));
        }

        let status = child.wait().await;

        if let Some(tmp) = temp_override_path {
            let _ = std::fs::remove_file(tmp);
        }

        match status {
            Ok(s) if s.success() => {
                let _ = state.db.with_conn(|conn| {
                    conn.execute(
                        "UPDATE deployments SET status = 'success', build_log = ?2, updated_at = datetime('now') WHERE id = ?1",
                        rusqlite::params![deployment_id, build_log],
                    )?;
                    Ok(())
                });
                yield Ok(Event::default().event("done").data("Compose finished successfully"));
            }
            Ok(s) => {
                let _ = state.db.with_conn(|conn| {
                    conn.execute(
                        "UPDATE deployments SET status = 'failed', build_log = ?2, updated_at = datetime('now') WHERE id = ?1",
                        rusqlite::params![deployment_id, build_log],
                    )?;
                    Ok(())
                });
                yield Ok(Event::default().event("error").data(format!("Compose exited with code {}", s.code().unwrap_or(-1))));
            }
            Err(e) => {
                let _ = state.db.with_conn(|conn| {
                    conn.execute(
                        "UPDATE deployments SET status = 'failed', build_log = ?2, updated_at = datetime('now') WHERE id = ?1",
                        rusqlite::params![deployment_id, build_log],
                    )?;
                    Ok(())
                });
                yield Ok(Event::default().event("error").data(format!("Process error: {}", e)));
            }
        }
    };

    Ok(Sse::new(stream))
}

async fn save_compose_override(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(i64, String)>,
    Json(body): Json<SaveComposeOverrideRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid filename"})),
        ));
    }

    let ovr_dir = override_dir(&state.config.data_dir, id);
    std::fs::create_dir_all(&ovr_dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
    })?;

    let path = format!("{}/{}", ovr_dir, filename);
    std::fs::write(&path, &body.content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
    })?;

    Ok(Json(json!({"message": "Override saved"})))
}

async fn reset_compose_override(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(i64, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid filename"})),
        ));
    }

    let path = format!("{}/{}", override_dir(&state.config.data_dir, id), filename);
    let _ = std::fs::remove_file(&path);

    Ok(Json(json!({"message": "Override reset"})))
}

async fn pull_repository(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let repo = get_repository(State(state.clone()), Path(id)).await?.0;
    
    let repo_dir = format!("{}/repos/{}", state.config.data_dir, id);
    if !std::path::Path::new(&repo_dir).join(".git").exists() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "Repository not cloned"}))));
    }
    
    let mut cmd = tokio::process::Command::new(&state.config.git_bin);
    cmd.current_dir(&repo_dir);
    
    let mut temp_key_path = None;
    if let Some(ssh_key) = &repo.ssh_password {
        if !ssh_key.trim().is_empty() {
            let key_path = format!("{}/repos/{}_id_rsa", state.config.data_dir, id);
            std::fs::write(&key_path, ssh_key.trim()).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&key_path).unwrap().permissions();
                perms.set_mode(0o600);
                std::fs::set_permissions(&key_path, perms).unwrap();
            }
            cmd.env("GIT_SSH_COMMAND", format!("ssh -i {} -o StrictHostKeyChecking=no", key_path));
            temp_key_path = Some(key_path);
        } else {
            cmd.env("GIT_SSH_COMMAND", "ssh -o StrictHostKeyChecking=no");
        }
    } else {
        cmd.env("GIT_SSH_COMMAND", "ssh -o StrictHostKeyChecking=no");
    }

    let output = cmd
        .arg("pull")
        .output()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
        
    if let Some(key_path) = temp_key_path {
        let _ = std::fs::remove_file(key_path);
    }
        
    if !output.status.success() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": String::from_utf8_lossy(&output.stderr).to_string()}))));
    }
    
    Ok(Json(json!({"message": "Repository pulled successfully"})))
}

async fn fetch_repository(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let repo = get_repository(State(state.clone()), Path(id)).await?.0;
    
    let repo_dir = format!("{}/repos/{}", state.config.data_dir, id);
    if !std::path::Path::new(&repo_dir).join(".git").exists() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "Repository not cloned"}))));
    }
    
    let mut cmd = tokio::process::Command::new(&state.config.git_bin);
    cmd.current_dir(&repo_dir);
    
    let mut temp_key_path = None;
    if let Some(ssh_key) = &repo.ssh_password {
        if !ssh_key.trim().is_empty() {
            let key_path = format!("{}/repos/{}_id_rsa", state.config.data_dir, id);
            std::fs::write(&key_path, ssh_key.trim()).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&key_path).unwrap().permissions();
                perms.set_mode(0o600);
                std::fs::set_permissions(&key_path, perms).unwrap();
            }
            cmd.env("GIT_SSH_COMMAND", format!("ssh -i {} -o StrictHostKeyChecking=no", key_path));
            temp_key_path = Some(key_path);
        } else {
            cmd.env("GIT_SSH_COMMAND", "ssh -o StrictHostKeyChecking=no");
        }
    } else {
        cmd.env("GIT_SSH_COMMAND", "ssh -o StrictHostKeyChecking=no");
    }

    let output = cmd
        .arg("fetch")
        .output()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
        
    if let Some(key_path) = temp_key_path {
        let _ = std::fs::remove_file(key_path);
    }
        
    if !output.status.success() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": String::from_utf8_lossy(&output.stderr).to_string()}))));
    }
    
    Ok(Json(json!({"message": "Repository fetched successfully"})))
}
