use serde::{Deserialize, Serialize};

/// How systemd should restart the workload.
///
/// `#[default]` on the variant rather than a manual `impl Default` — clippy's
/// `derivable_impls` lint rejects the hand-written version under `-D warnings`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestartPolicy {
    #[default]
    Always,
    OnFailure,
    No,
}

impl RestartPolicy {
    pub fn as_systemd(&self) -> &'static str {
        match self {
            RestartPolicy::Always => "always",
            RestartPolicy::OnFailure => "on-failure",
            RestartPolicy::No => "no",
        }
    }
}

/// Declarative description of one workload. The source of truth; unit files are derived.
///
/// `secrets` holds secret *names* only — values live in `podman secret` and never appear here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadSpec {
    pub name: String,
    pub image: String,
    pub command: Option<Vec<String>>,
    pub env: Vec<(String, String)>,
    pub ports: Vec<String>,
    pub volumes: Vec<String>,
    pub secrets: Vec<String>,
    pub memory_max: Option<String>,
    pub health_cmd: Option<String>,
    pub restart_policy: RestartPolicy,
}

impl WorkloadSpec {
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
            ..Default::default()
        }
    }

    /// Filesystem- and systemd-safe identifier derived from the name.
    pub fn slug(&self) -> String {
        slug(&self.name)
    }
}

/// Lowercase, collapse separators to `-`, drop anything else.
pub fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if (ch == ' ' || ch == '-' || ch == '_') && !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_lowercases_and_replaces_separators() {
        assert_eq!(slug("My App"), "my-app");
        assert_eq!(slug("pbrain_api"), "pbrain-api");
        assert_eq!(slug("Web--Server "), "web-server");
    }

    #[test]
    fn slug_strips_disallowed_characters() {
        assert_eq!(slug("app@v1.2"), "appv12");
    }

    #[test]
    fn restart_policy_maps_to_systemd_values() {
        assert_eq!(RestartPolicy::Always.as_systemd(), "always");
        assert_eq!(RestartPolicy::OnFailure.as_systemd(), "on-failure");
        assert_eq!(RestartPolicy::No.as_systemd(), "no");
    }

    #[test]
    fn new_spec_defaults_to_restart_always_and_empty_collections() {
        let spec = WorkloadSpec::new("pbrain", "docker.io/library/node:22-alpine");
        assert_eq!(spec.name, "pbrain");
        assert_eq!(spec.restart_policy, RestartPolicy::Always);
        assert!(spec.env.is_empty());
        assert!(spec.secrets.is_empty());
        assert!(spec.memory_max.is_none());
    }

    #[test]
    fn spec_round_trips_through_json() {
        let mut spec = WorkloadSpec::new("pbrain", "node:22-alpine");
        spec.ports.push("3000:3000".to_string());
        spec.secrets.push("db-password".to_string());
        let json = serde_json::to_string(&spec).expect("serialize");
        let back: WorkloadSpec = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(spec, back);
    }
}
