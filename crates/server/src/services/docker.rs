use anyhow::Result;
use bollard::container::{
    ListContainersOptions, LogsOptions, RemoveContainerOptions, RestartContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::Docker;
use std::collections::HashMap;

use crate::db::models::{ContainerInfo, PortMapping};

pub struct DockerService {
    client: Docker,
}

impl DockerService {
    pub async fn new() -> Result<Self> {
        let client = Docker::connect_with_local_defaults()?;
        // Verify connection
        client.ping().await?;
        Ok(Self { client })
    }

    pub async fn list_containers(&self, all: bool) -> Result<Vec<ContainerInfo>> {
        let mut filters = HashMap::new();
        if !all {
            filters.insert("status".to_string(), vec!["running".to_string()]);
        }

        let options = ListContainersOptions {
            all,
            filters,
            ..Default::default()
        };

        let containers = self.client.list_containers(Some(options)).await?;

        let result: Vec<ContainerInfo> = containers
            .into_iter()
            .map(|c| {
                let ports = c
                    .ports
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| PortMapping {
                        private_port: p.private_port,
                        public_port: p.public_port,
                        port_type: p.typ.map(|t| format!("{:?}", t)).unwrap_or_default(),
                    })
                    .collect();

                let name = c
                    .names
                    .and_then(|n| n.first().cloned())
                    .unwrap_or_default()
                    .trim_start_matches('/')
                    .to_string();

                ContainerInfo {
                    id: c.id.unwrap_or_default(),
                    name,
                    image: c.image.unwrap_or_default(),
                    status: c.status.unwrap_or_default(),
                    state: c.state.unwrap_or_default(),
                    ports,
                    created: c.created.unwrap_or(0),
                }
            })
            .collect();

        Ok(result)
    }

    pub async fn start_container(&self, id: &str) -> Result<()> {
        self.client
            .start_container(id, None::<StartContainerOptions<String>>)
            .await?;
        Ok(())
    }

    pub async fn stop_container(&self, id: &str) -> Result<()> {
        self.client
            .stop_container(id, Some(StopContainerOptions { t: 10 }))
            .await?;
        Ok(())
    }

    pub async fn restart_container(&self, id: &str) -> Result<()> {
        self.client
            .restart_container(id, Some(RestartContainerOptions { t: 10 }))
            .await?;
        Ok(())
    }

    pub async fn remove_container(&self, id: &str, force: bool) -> Result<()> {
        self.client
            .remove_container(
                id,
                Some(RemoveContainerOptions {
                    force,
                    ..Default::default()
                }),
            )
            .await?;
        Ok(())
    }

    pub async fn get_container_logs(&self, id: &str, tail: usize) -> Result<String> {
        use bollard::container::LogOutput;
        use futures_util::TryStreamExt;

        let options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            tail: tail.to_string(),
            ..Default::default()
        };

        let mut stream = self.client.logs(id, Some(options));
        let mut output = String::new();

        while let Some(log) = stream.try_next().await? {
            match log {
                LogOutput::StdOut { message } | LogOutput::StdErr { message } => {
                    output.push_str(&String::from_utf8_lossy(&message));
                }
                _ => {}
            }
        }

        Ok(output)
    }

    pub async fn inspect_container(
        &self,
        id: &str,
    ) -> Result<bollard::models::ContainerInspectResponse> {
        let info = self.client.inspect_container(id, None).await?;
        Ok(info)
    }
}
