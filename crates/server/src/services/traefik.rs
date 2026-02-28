use anyhow::Result;
use bollard::container::{Config, CreateContainerOptions, ListContainersOptions, StartContainerOptions};
use bollard::models::{HostConfig, PortBinding, RestartPolicy, RestartPolicyNameEnum};
use bollard::network::{ConnectNetworkOptions, CreateNetworkOptions};
use bollard::Docker;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const TRAEFIK_NETWORK: &str = "dockyy-net";
pub const TRAEFIK_CONTAINER: &str = "dockyy-traefik";
const TRAEFIK_IMAGE: &str = "traefik:v3.3";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProxyRoute {
    pub container_id: String,
    pub container_name: String,
    pub domain: String,
    pub port: u16,
    pub status: String,
}

pub struct TraefikService {
    docker: Docker,
}

impl TraefikService {
    pub fn new(docker: Docker) -> Self {
        Self { docker }
    }

    /// Create the shared Docker network if it does not exist.
    pub async fn ensure_network(&self) -> Result<()> {
        let networks = self.docker.list_networks::<String>(None).await?;
        let exists = networks
            .iter()
            .any(|n| n.name.as_deref() == Some(TRAEFIK_NETWORK));

        if !exists {
            tracing::info!("Creating Docker network '{}'", TRAEFIK_NETWORK);
            self.docker
                .create_network(CreateNetworkOptions {
                    name: TRAEFIK_NETWORK,
                    ..Default::default()
                })
                .await?;
            tracing::info!("Docker network '{}' created", TRAEFIK_NETWORK);
        }
        Ok(())
    }

    /// Ensure the Traefik container is running, creating it if needed.
    pub async fn ensure_traefik(&self, http_port: u16) -> Result<()> {
        self.ensure_network().await?;

        let mut filters = HashMap::new();
        filters.insert("name".to_string(), vec![TRAEFIK_CONTAINER.to_string()]);
        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters,
                ..Default::default()
            }))
            .await?;

        if let Some(c) = containers.first() {
            let state = c.state.as_deref().unwrap_or("");
            if state == "running" {
                tracing::debug!("Traefik container already running");
                return Ok(());
            }
            // Container exists but is stopped — start it
            let id = c.id.as_deref().unwrap_or(TRAEFIK_CONTAINER);
            tracing::info!("Starting existing Traefik container");
            self.docker
                .start_container(id, None::<StartContainerOptions<String>>)
                .await?;
            return Ok(());
        }

        // Create and start a fresh Traefik container
        tracing::info!(
            "Creating Traefik container '{}' on port {}",
            TRAEFIK_CONTAINER,
            http_port
        );

        let mut port_bindings = HashMap::new();
        port_bindings.insert(
            "80/tcp".to_string(),
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(http_port.to_string()),
            }]),
        );
        // Traefik dashboard exposed only on localhost
        port_bindings.insert(
            "8080/tcp".to_string(),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: Some("8080".to_string()),
            }]),
        );

        let cmd = vec![
            "--api.insecure=true".to_string(),
            "--providers.docker=true".to_string(),
            format!("--providers.docker.network={}", TRAEFIK_NETWORK),
            "--providers.docker.exposedbydefault=false".to_string(),
            "--entrypoints.web.address=:80".to_string(),
        ];

        let mut labels = HashMap::new();
        labels.insert("dockyy.managed".to_string(), "true".to_string());

        let config = Config {
            image: Some(TRAEFIK_IMAGE.to_string()),
            cmd: Some(cmd),
            labels: Some(labels),
            host_config: Some(HostConfig {
                binds: Some(vec![
                    "/var/run/docker.sock:/var/run/docker.sock:ro".to_string()
                ]),
                port_bindings: Some(port_bindings),
                restart_policy: Some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::ALWAYS),
                    ..Default::default()
                }),
                network_mode: Some(TRAEFIK_NETWORK.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        self.docker
            .create_container(
                Some(CreateContainerOptions {
                    name: TRAEFIK_CONTAINER,
                    platform: None,
                }),
                config,
            )
            .await?;

        self.docker
            .start_container(TRAEFIK_CONTAINER, None::<StartContainerOptions<String>>)
            .await?;

        tracing::info!("Traefik container started");
        Ok(())
    }

    /// Generate Traefik labels for a container so it gets routed by domain.
    ///
    /// `router_name` must be unique per container (use the container name).
    pub fn container_labels(
        router_name: &str,
        domain: &str,
        port: u16,
    ) -> HashMap<String, String> {
        let mut labels = HashMap::new();
        labels.insert("traefik.enable".to_string(), "true".to_string());
        labels.insert(
            format!("traefik.http.routers.{}.rule", router_name),
            format!("Host(`{}`)", domain),
        );
        labels.insert(
            format!("traefik.http.routers.{}.entrypoints", router_name),
            "web".to_string(),
        );
        labels.insert(
            format!(
                "traefik.http.services.{}.loadbalancer.server.port",
                router_name
            ),
            port.to_string(),
        );
        labels
    }

    /// Connect an already-running container to the shared dockyy-net network.
    pub async fn connect_container(&self, container_id: &str) -> Result<()> {
        self.docker
            .connect_network(
                TRAEFIK_NETWORK,
                ConnectNetworkOptions {
                    container: container_id,
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }

    /// List all running containers that have Traefik routing enabled.
    pub async fn list_routes(&self) -> Result<Vec<ProxyRoute>> {
        let mut filters = HashMap::new();
        filters.insert("status".to_string(), vec!["running".to_string()]);
        filters.insert(
            "label".to_string(),
            vec!["traefik.enable=true".to_string()],
        );

        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: false,
                filters,
                ..Default::default()
            }))
            .await?;

        let mut routes = Vec::new();
        for c in containers {
            let labels = c.labels.unwrap_or_default();
            let name = c
                .names
                .and_then(|n| n.first().cloned())
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_string();

            // Parse domain from: traefik.http.routers.<name>.rule = Host(`domain`)
            let domain = labels
                .iter()
                .find(|(k, _)| k.ends_with(".rule"))
                .and_then(|(_, v)| {
                    v.strip_prefix("Host(`")
                        .and_then(|s| s.strip_suffix("`)"))
                        .map(|s| s.to_string())
                });

            let port = labels
                .iter()
                .find(|(k, _)| k.ends_with(".loadbalancer.server.port"))
                .and_then(|(_, v)| v.parse::<u16>().ok())
                .unwrap_or(80);

            if let Some(domain) = domain {
                routes.push(ProxyRoute {
                    container_id: c.id.unwrap_or_default(),
                    container_name: name,
                    domain,
                    port,
                    status: c.status.unwrap_or_default(),
                });
            }
        }

        Ok(routes)
    }

    /// Check whether the Traefik container is currently running.
    pub async fn is_running(&self) -> Result<bool> {
        let mut filters = HashMap::new();
        filters.insert("name".to_string(), vec![TRAEFIK_CONTAINER.to_string()]);
        filters.insert("status".to_string(), vec!["running".to_string()]);

        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                all: false,
                filters,
                ..Default::default()
            }))
            .await?;

        Ok(!containers.is_empty())
    }
}
