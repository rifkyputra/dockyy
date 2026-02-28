use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Repository {
    pub id: i64,
    pub name: String,
    pub owner: String,
    pub url: String,
    pub description: Option<String>,
    pub webhook_url: Option<String>,
    pub filesystem_path: Option<String>,
    pub ssh_password: Option<String>,
    pub is_private: bool,
    pub default_branch: String,
    /// Domain used for automatic reverse-proxy routing via Traefik.
    pub domain: Option<String>,
    /// Internal port the container listens on (used by Traefik; default 3000).
    pub proxy_port: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRepository {
    pub name: String,
    pub owner: String,
    pub url: String,
    pub description: Option<String>,
    pub webhook_url: Option<String>,
    pub filesystem_path: Option<String>,
    pub ssh_password: Option<String>,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default = "default_branch")]
    pub default_branch: String,
    pub domain: Option<String>,
    pub proxy_port: Option<i64>,
}

fn default_branch() -> String {
    "main".into()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateRepository {
    pub name: Option<String>,
    pub owner: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub webhook_url: Option<String>,
    pub filesystem_path: Option<String>,
    pub ssh_password: Option<String>,
    pub is_private: Option<bool>,
    pub default_branch: Option<String>,
    pub domain: Option<String>,
    pub proxy_port: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Deployment {
    pub id: i64,
    pub repo_id: i64,
    pub status: String,
    pub commit_sha: Option<String>,
    pub image_name: Option<String>,
    pub container_id: Option<String>,
    pub domain: Option<String>,
    pub port: Option<i64>,
    pub build_log: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Job {
    pub id: i64,
    pub job_type: String,
    pub payload: String,
    pub status: String,
    pub result: Option<String>,
    pub attempts: i64,
    pub max_attempts: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub state: String,
    pub ports: Vec<PortMapping>,
    pub created: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PortMapping {
    pub private_port: u16,
    pub public_port: Option<u16>,
    pub port_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnvVar {
    pub id: i64,
    pub repo_id: i64,
    pub key: String,
    pub value: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpsertEnvVar {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateEnvVarValue {
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportFromCompose {
    pub compose_file: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
}
