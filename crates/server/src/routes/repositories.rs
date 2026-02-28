use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::db::models::{CreateRepository, Repository, UpdateRepository};
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

async fn get_compose_files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let repo_dir = format!("{}/repos/{}", state.config.data_dir, id);
    let path = std::path::Path::new(&repo_dir);
    
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
                    files.push(json!({
                        "path": name,
                        "content": c
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
    
    let mut cmd = tokio::process::Command::new("git");
    
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
        .arg("clone")
        .arg(&repo.url)
        .arg(&repo_dir)
        .output()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
        
    if let Some(key_path) = temp_key_path {
        let _ = std::fs::remove_file(key_path);
    }
        
    if !output.status.success() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": String::from_utf8_lossy(&output.stderr).to_string()}))));
    }
    
    Ok(Json(json!({"message": "Repository cloned successfully"})))
}

async fn docker_compose_up(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let repo = get_repository(State(state.clone()), Path(id)).await?.0;
    let repo_dir = format!("{}/repos/{}", state.config.data_dir, id);
    
    let container_name = format!("dockyy-{}", repo.name.to_lowercase().replace("/", "-"));
    
    let output = tokio::process::Command::new("docker")
        .arg("compose")
        .arg("-p")
        .arg(&container_name)
        .arg("up")
        .arg("-d")
        .arg("--build")
        .current_dir(&repo_dir)
        .output()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
        
    if !output.status.success() {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": String::from_utf8_lossy(&output.stderr).to_string()}))));
    }
    
    Ok(Json(json!({"message": "Deployment started with docker-compose"})))
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
    
    let mut cmd = tokio::process::Command::new("git");
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
    
    let mut cmd = tokio::process::Command::new("git");
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
