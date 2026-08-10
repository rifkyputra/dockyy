use anyhow::Result;

use crate::spec::{slug, WorkloadSpec};
use crate::workloads::paths::UNIT_PREFIX;

/// Marker identifying a unit file kuadrat generated and may overwrite.
pub const MANAGED_MARKER: &str = "# kuadrat-managed: true";

/// Container name kuadrat assigns to a workload. Shares the unit-file prefix, so the
/// container, the unit file, and the systemd service all carry the same namespace.
pub fn container_name(spec: &WorkloadSpec) -> String {
    format!("{UNIT_PREFIX}{}", slug(&spec.name))
}

/// Escape a literal `%` so systemd/Quadlet does not treat it as a specifier.
fn escape_percent(s: &str) -> String {
    s.replace('%', "%%")
}

/// Quote one `Exec=` argument per systemd's exec syntax.
///
/// systemd splits `Exec=` on whitespace, so an argument containing a space becomes two
/// arguments unless it is quoted. Inside double quotes systemd honours C-style escapes,
/// so `\` and `"` must be escaped.
fn quote_exec_arg(arg: &str) -> String {
    let needs_quoting = arg.is_empty()
        || arg
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'));

    if !needs_quoting {
        return arg.to_string();
    }
    let escaped = arg.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Render a spec to Quadlet `.container` unit text. Pure — no I/O.
///
/// Validates first: a spec that would inject directives never gets rendered, so no caller
/// can write one to disk by forgetting to check.
pub fn render(spec: &WorkloadSpec) -> Result<String> {
    spec.validate()?;

    let mut out = String::new();

    out.push_str(MANAGED_MARKER);
    out.push('\n');

    out.push_str("[Unit]\n");
    out.push_str(&format!("Description=kuadrat workload {}\n\n", spec.name));

    out.push_str("[Container]\n");
    out.push_str(&format!("Image={}\n", spec.image));
    out.push_str(&format!("ContainerName={}\n", container_name(spec)));
    for port in &spec.ports {
        out.push_str(&format!("PublishPort={port}\n"));
    }
    for volume in &spec.volumes {
        out.push_str(&format!("Volume={}\n", escape_percent(volume)));
    }
    for (key, value) in &spec.env {
        out.push_str(&format!(
            "Environment={}={}\n",
            escape_percent(key),
            escape_percent(value)
        ));
    }
    for secret in &spec.secrets {
        out.push_str(&format!("Secret={secret}\n"));
    }
    if let Some(health) = &spec.health_cmd {
        out.push_str(&format!("HealthCmd={health}\n"));
    }
    if let Some(command) = &spec.command {
        let argv: Vec<String> = command
            .iter()
            .map(|a| quote_exec_arg(&escape_percent(a)))
            .collect();
        out.push_str(&format!("Exec={}\n", argv.join(" ")));
    }
    out.push('\n');

    out.push_str("[Service]\n");
    out.push_str(&format!("Restart={}\n", spec.restart_policy.as_systemd()));
    if let Some(memory) = &spec.memory_max {
        out.push_str(&format!("MemoryMax={memory}\n"));
    }
    out.push('\n');

    out.push_str("[Install]\nWantedBy=multi-user.target\n");

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{RestartPolicy, WorkloadSpec};

    #[test]
    fn renders_minimal_spec() {
        let spec = WorkloadSpec::new("pbrain", "docker.io/library/node:22-alpine");
        let expected = include_str!("../../tests/golden/minimal.container");
        assert_eq!(render(&spec).expect("render"), expected);
    }

    #[test]
    fn renders_full_spec() {
        let mut spec = WorkloadSpec::new("pbrain api", "docker.io/library/node:22-alpine");
        spec.ports = vec!["3000:3000".into(), "9229:9229".into()];
        spec.volumes = vec!["/srv/pbrain:/app:Z".into()];
        spec.env = vec![
            ("NODE_ENV".into(), "production".into()),
            ("PORT".into(), "3000".into()),
        ];
        spec.secrets = vec!["db-password".into()];
        spec.health_cmd = Some("curl -fsS http://localhost:3000/health".into());
        spec.command = Some(vec![
            "node".into(),
            "server.js".into(),
            "--port".into(),
            "3000".into(),
        ]);
        spec.memory_max = Some("512M".into());
        spec.restart_policy = RestartPolicy::OnFailure;

        let expected = include_str!("../../tests/golden/full.container");
        assert_eq!(render(&spec).expect("render"), expected);
    }

    #[test]
    fn renders_spec_with_spaced_command_arguments() {
        let mut spec = WorkloadSpec::new("shell", "docker.io/library/alpine:3.20");
        spec.command = Some(vec![
            "sh".into(),
            "-c".into(),
            "echo hello world".into(),
            "say \"hi\"".into(),
        ]);

        let expected = include_str!("../../tests/golden/spaced-exec.container");
        assert_eq!(render(&spec).expect("render"), expected);
    }

    #[test]
    fn exec_quotes_only_the_arguments_that_need_it() {
        assert_eq!(quote_exec_arg("node"), "node");
        assert_eq!(quote_exec_arg("--port"), "--port");
        assert_eq!(quote_exec_arg("echo hello world"), "\"echo hello world\"");
        assert_eq!(quote_exec_arg("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_exec_arg("a\\b"), "\"a\\\\b\"");
        assert_eq!(quote_exec_arg("it's"), "\"it's\"");
        assert_eq!(quote_exec_arg(""), "\"\"");
    }

    #[test]
    fn render_refuses_a_spec_that_would_inject_directives() {
        let mut spec = WorkloadSpec::new("pbrain", "alpine");
        spec.env = vec![("X".into(), "1\nUser=root".into())];

        let err = render(&spec).expect_err("render validates");
        assert!(err.to_string().contains("line break"), "{err}");
    }

    #[test]
    fn container_name_is_prefixed_slug() {
        let spec = WorkloadSpec::new("My App", "alpine");
        assert_eq!(container_name(&spec), "kuadrat-my-app");
    }

    #[test]
    fn rendered_unit_always_carries_the_managed_marker() {
        let spec = WorkloadSpec::new("x", "alpine");
        assert!(render(&spec).expect("render").starts_with(MANAGED_MARKER));
    }

    #[test]
    fn a_percent_in_an_env_value_is_escaped() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.env = vec![("PW".into(), "a%b".into())];
        let unit = render(&spec).expect("render");
        assert!(unit.contains("Environment=PW=a%%b"), "unit was:\n{unit}");
    }

    #[test]
    fn a_percent_in_an_exec_arg_is_escaped() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.command = Some(vec!["printf".into(), "100%".into()]);
        let unit = render(&spec).expect("render");
        assert!(unit.contains("100%%"), "unit was:\n{unit}");
    }

    #[test]
    fn a_percent_in_a_volume_is_escaped() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.volumes = vec!["/data/50%:/x".into()];
        let unit = render(&spec).expect("render");
        assert!(unit.contains("Volume=/data/50%%:/x"), "unit was:\n{unit}");
    }
}
