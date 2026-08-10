use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// A public route: a domain reverse-proxied to a local port. Rendered into a
/// Caddy fragment by the `gateway` module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub domain: String,
    pub port: u16,
}

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
    pub route: Option<Route>,
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

    /// Reject anything that would not survive being written into a Quadlet unit file.
    ///
    /// A unit file is line-oriented: one `Key=value` per line. A `\n` or `\r` inside any
    /// rendered value therefore does not corrupt the file, it *extends* it with directives
    /// nobody wrote — including `Secret=` and `User=`. Called at the top of `render`, so no
    /// unvalidated spec can reach disk.
    ///
    /// Error messages name the offending **field**, never its value: an environment value or
    /// a command argument may itself carry a secret, and errors get logged.
    pub fn validate(&self) -> Result<()> {
        if self.slug().is_empty() {
            bail!(
                "workload name {:?} yields an empty identifier; it needs at least one \
                 letter or digit",
                self.name
            );
        }

        single_line("name", &self.name)?;
        single_line("image", &self.image)?;
        for (i, port) in self.ports.iter().enumerate() {
            single_line(&format!("ports[{i}]"), port)?;
        }
        for (i, volume) in self.volumes.iter().enumerate() {
            single_line(&format!("volumes[{i}]"), volume)?;
        }
        for (key, value) in &self.env {
            single_line(&format!("env key {key:?}"), key)?;
            single_line(&format!("env value for {key:?}"), value)?;
        }
        for (i, secret) in self.secrets.iter().enumerate() {
            single_line(&format!("secrets[{i}]"), secret)?;
        }
        if let Some(memory) = &self.memory_max {
            single_line("memory_max", memory)?;
        }
        if let Some(health) = &self.health_cmd {
            single_line("health_cmd", health)?;
        }
        for (i, arg) in self.command.iter().flatten().enumerate() {
            single_line(&format!("command[{i}]"), arg)?;
        }

        if self.route.is_some() && self.health_cmd.is_none() {
            bail!(
                "workload {:?} declares a route but no health_cmd: public traffic \
                 must not reach a service with no readiness check",
                self.name
            );
        }
        if let Some(route) = &self.route {
            single_line("route domain", &route.domain)?;
        }

        Ok(())
    }
}

/// Reject a line break in a field that renders as one unit-file line. Value never echoed.
fn single_line(field: &str, value: &str) -> Result<()> {
    if value.contains('\n') || value.contains('\r') {
        bail!("{field} contains a line break, which would inject Quadlet directives");
    }
    Ok(())
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
    fn validate_rejects_a_newline_in_an_env_value() {
        let mut spec = WorkloadSpec::new("pbrain", "alpine");
        spec.env = vec![(
            "REPLICAS".into(),
            "1\nSecret=db-password,type=env\nUser=root".into(),
        )];

        let err = spec.validate().expect_err("newline is rejected");
        let msg = err.to_string();
        assert!(msg.contains("env value for \"REPLICAS\""), "{msg}");
        assert!(msg.contains("line break"), "{msg}");
        // The value may be a secret: it must not appear in the error.
        assert!(!msg.contains("db-password"), "{msg}");
        assert!(!msg.contains("User=root"), "{msg}");
    }

    #[test]
    fn validate_rejects_line_breaks_in_every_rendered_field() {
        /// A field label and a mutation that puts a line break into that field.
        type Case = (&'static str, fn(&mut WorkloadSpec));

        let fields: Vec<Case> = vec![
            ("name", |s| s.name = "ok\nbad".into()),
            ("image", |s| s.image = "alpine\nbad".into()),
            ("ports", |s| s.ports = vec!["80:80\nbad".into()]),
            ("volumes", |s| s.volumes = vec!["/a:/b\nbad".into()]),
            ("secrets", |s| s.secrets = vec!["tok\nbad".into()]),
            ("memory_max", |s| s.memory_max = Some("1G\nbad".into())),
            ("health_cmd", |s| s.health_cmd = Some("true\nbad".into())),
            ("command", |s| s.command = Some(vec!["sh\nbad".into()])),
            ("env key", |s| s.env = vec![("K\nbad".into(), "v".into())]),
        ];

        for (label, mutate) in fields {
            let mut spec = WorkloadSpec::new("pbrain", "alpine");
            mutate(&mut spec);
            let err = spec.validate().expect_err(label);
            assert!(err.to_string().contains("line break"), "{label}: {err}");
        }
    }

    #[test]
    fn validate_rejects_a_name_with_an_empty_slug() {
        for name in ["@@@", "---", "", "   ", "!"] {
            let spec = WorkloadSpec::new(name, "alpine");
            assert_eq!(spec.slug(), "");
            let err = spec.validate().expect_err(name);
            assert!(err.to_string().contains("empty identifier"), "{err}");
        }
    }

    #[test]
    fn validate_accepts_an_ordinary_spec() {
        let mut spec = WorkloadSpec::new("pbrain api", "alpine");
        spec.env = vec![("NODE_ENV".into(), "production".into())];
        spec.command = Some(vec!["sh".into(), "-c".into(), "echo hello world".into()]);
        spec.validate().expect("ordinary spec is valid");
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

    #[test]
    fn validate_rejects_a_route_without_a_health_cmd() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.route = Some(Route {
            domain: "example.com".into(),
            port: 3000,
        });
        let err = spec.validate().unwrap_err();
        assert!(err.to_string().contains("health_cmd"), "message was: {err}");
    }

    #[test]
    fn validate_accepts_a_route_with_a_health_cmd() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.route = Some(Route {
            domain: "example.com".into(),
            port: 3000,
        });
        spec.health_cmd = Some("curl -fsS http://localhost:3000/health".into());
        spec.validate().expect("valid");
    }

    #[test]
    fn a_spec_with_a_route_round_trips_through_json() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.route = Some(Route {
            domain: "example.com".into(),
            port: 3000,
        });
        let json = serde_json::to_string(&spec).expect("serialize");
        let back: WorkloadSpec = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(spec, back);
    }
}
