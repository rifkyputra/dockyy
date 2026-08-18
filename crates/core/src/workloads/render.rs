use std::time::Duration;

use anyhow::Result;

use crate::spec::{slug, ScheduledTask, WorkloadSpec};
use crate::workloads::paths::{task_unit_name, UNIT_PREFIX};

/// Marker identifying a unit file kuadrat generated and may overwrite.
pub const MANAGED_MARKER: &str = "# kuadrat-managed: true";

/// Per-attempt timeout for a `podman healthcheck run` call, shared with
/// `deploy::health`'s poll loop. Lives here (rather than in `deploy::health`)
/// because `deploy` already depends on `workloads`, not the other way round;
/// `deploy::health` imports this constant so podman's own `HealthTimeout=`
/// and kuadrat's `tokio::time::timeout` never drift apart.
pub const HEALTH_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

/// Container name kuadrat assigns to a workload. Shares the unit-file prefix, so the
/// container, the unit file, and the systemd service all carry the same namespace.
pub fn container_name(spec: &WorkloadSpec) -> String {
    format!("{UNIT_PREFIX}{}", slug(&spec.name))
}

/// Escape a literal `%` so systemd/Quadlet does not treat it as a specifier.
fn escape_percent(s: &str) -> String {
    s.replace('%', "%%")
}

/// Quote one systemd word — an `Exec=` argument or a whole `Environment=`
/// assignment.
///
/// systemd splits both directives on whitespace: `Exec=` into argv, and
/// `Environment=` into separate `KEY=VALUE` assignments. Either way a word
/// containing a space becomes two words unless it is quoted. Inside double
/// quotes systemd honours C-style escapes, so `\` and `"` must be escaped.
///
/// For `Environment=` the **entire** assignment is quoted, not just the value:
/// `Environment="GREETING=hello world"` is the form systemd documents.
fn quote_word(arg: &str) -> String {
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
        // Percent-escape first, then quote: quoting only escapes `\` and `"`,
        // so it cannot disturb an already-doubled `%%`.
        let assignment = format!("{}={}", escape_percent(key), escape_percent(value));
        out.push_str(&format!("Environment={}\n", quote_word(&assignment)));
    }
    for secret in &spec.secrets {
        out.push_str(&format!("Secret={secret}\n"));
    }
    if let Some(health) = &spec.health_cmd {
        out.push_str(&format!("HealthCmd={health}\n"));
        // Keep podman's own limit in step with kuadrat's per-attempt timeout
        // (deploy::health), so a hanging health command is cut off at the
        // same point on both sides rather than podman's default governing.
        out.push_str(&format!(
            "HealthTimeout={}s\n",
            HEALTH_ATTEMPT_TIMEOUT.as_secs()
        ));
    }
    if let Some(command) = &spec.command {
        let argv: Vec<String> = command
            .iter()
            .map(|a| quote_word(&escape_percent(a)))
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

/// Render one scheduled task as a Quadlet oneshot `.container`. Pure — no I/O.
///
/// The task gets the app's image, env, and secrets and its own `Exec` —
/// never ports, volumes, a route, or a healthcheck: the timer is its only
/// trigger and its exit code its only result. No `[Install]` section, so it
/// cannot start at boot; the timer activates the generated service by name.
pub fn render_task(spec: &WorkloadSpec, task: &ScheduledTask) -> Result<String> {
    spec.validate()?;

    let mut out = String::new();
    out.push_str(MANAGED_MARKER);
    out.push('\n');

    out.push_str("[Unit]\n");
    out.push_str(&format!(
        "Description=kuadrat task {} for {}\n\n",
        slug(&task.name),
        spec.name
    ));

    out.push_str("[Container]\n");
    out.push_str(&format!("Image={}\n", spec.image));
    out.push_str(&format!(
        "ContainerName={}\n",
        task_unit_name(&spec.name, &task.name)
    ));
    for (key, value) in &spec.env {
        let assignment = format!("{}={}", escape_percent(key), escape_percent(value));
        out.push_str(&format!("Environment={}\n", quote_word(&assignment)));
    }
    for secret in &spec.secrets {
        out.push_str(&format!("Secret={secret}\n"));
    }
    let argv: Vec<String> = task
        .command
        .iter()
        .map(|a| quote_word(&escape_percent(a)))
        .collect();
    out.push_str(&format!("Exec={}\n", argv.join(" ")));
    out.push('\n');

    out.push_str("[Service]\nType=oneshot\n");

    Ok(out)
}

/// Render one task's `.timer`. Pure — no I/O. Validates like every renderer,
/// so no unvalidated schedule reaches disk.
pub fn render_timer(spec: &WorkloadSpec, task: &ScheduledTask) -> Result<String> {
    spec.validate()?;

    let mut out = String::new();
    out.push_str(MANAGED_MARKER);
    out.push('\n');
    out.push_str("[Unit]\n");
    out.push_str(&format!(
        "Description=kuadrat timer {} for {}\n\n",
        slug(&task.name),
        spec.name
    ));
    out.push_str("[Timer]\n");
    out.push_str(&format!("OnCalendar={}\n", escape_percent(&task.schedule)));
    out.push_str("Persistent=true\n\n");
    out.push_str("[Install]\nWantedBy=timers.target\n");

    Ok(out)
}

#[cfg(test)]
mod tests_tasks {
    use super::*;
    use crate::spec::{ScheduledTask, WorkloadSpec};

    fn spec_with_task() -> (WorkloadSpec, ScheduledTask) {
        let mut spec = WorkloadSpec::new("web", "docker.io/library/alpine:3.20");
        spec.env = vec![("NODE_ENV".into(), "production".into())];
        spec.secrets = vec!["db-password".into()];
        let task = ScheduledTask {
            name: "Daily Cleanup".into(),
            schedule: "daily".into(),
            command: vec!["sh".into(), "-c".into(), "true".into()],
        };
        spec.tasks = vec![task.clone()];
        (spec, task)
    }

    #[test]
    fn renders_a_task_container_and_timer() {
        let (spec, task) = spec_with_task();
        assert_eq!(
            render_task(&spec, &task).expect("render"),
            include_str!("../../tests/golden/task.container")
        );
        assert_eq!(
            render_timer(&spec, &task).expect("render"),
            include_str!("../../tests/golden/task.timer")
        );
    }

    /// A task container must not start at boot, publish ports, or carry a
    /// healthcheck — the timer is its only trigger and its exit code is its
    /// only result.
    #[test]
    fn a_task_container_is_oneshot_and_routeless() {
        let (mut spec, task) = spec_with_task();
        spec.ports = vec!["3000:3000".into()];
        spec.health_cmd = Some("curl -fsS localhost".into());
        spec.route = None;
        let unit = render_task(&spec, &task).expect("render");
        assert!(unit.contains("Type=oneshot"), "{unit}");
        assert!(!unit.contains("PublishPort"), "{unit}");
        assert!(!unit.contains("HealthCmd"), "{unit}");
        assert!(!unit.contains("WantedBy=multi-user.target"), "{unit}");
    }

    #[test]
    fn rendered_task_files_always_carry_the_managed_marker() {
        let (spec, task) = spec_with_task();
        for text in [
            render_task(&spec, &task).expect("render"),
            render_timer(&spec, &task).expect("render"),
        ] {
            assert!(text.starts_with(MANAGED_MARKER), "{text}");
        }
    }
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
        assert_eq!(quote_word("node"), "node");
        assert_eq!(quote_word("--port"), "--port");
        assert_eq!(quote_word("echo hello world"), "\"echo hello world\"");
        assert_eq!(quote_word("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_word("a\\b"), "\"a\\\\b\"");
        assert_eq!(quote_word("it's"), "\"it's\"");
        assert_eq!(quote_word(""), "\"\"");
    }

    #[test]
    fn render_refuses_a_spec_that_would_inject_directives() {
        let mut spec = WorkloadSpec::new("pbrain", "alpine");
        spec.env = vec![("X".into(), "1\nUser=root".into())];

        let err = render(&spec).expect_err("render validates");
        assert!(err.to_string().contains("line break"), "{err}");
    }

    #[test]
    fn a_health_cmd_renders_a_matching_health_timeout() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.health_cmd = Some("curl -fsS localhost/health".into());
        let unit = render(&spec).expect("render");
        assert!(
            unit.contains("HealthCmd=curl -fsS localhost/health\n"),
            "unit was:\n{unit}"
        );
        assert!(
            unit.contains(&format!(
                "HealthTimeout={}s\n",
                HEALTH_ATTEMPT_TIMEOUT.as_secs()
            )),
            "unit was:\n{unit}"
        );
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

    /// The bug this pins: `Environment=` splits on whitespace exactly like
    /// `Exec=` does, so an unquoted two-word value silently truncated at the
    /// first space. Found by deploying examples/hello-py, whose GREETING was
    /// "hello from kuadrat" and arrived in the container as "hello".
    #[test]
    fn an_env_value_with_spaces_is_quoted() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.env = vec![("GREETING".into(), "hello from kuadrat".into())];
        let unit = render(&spec).expect("render");
        assert!(
            unit.contains("Environment=\"GREETING=hello from kuadrat\""),
            "unit was:\n{unit}"
        );
    }

    #[test]
    fn an_env_value_without_spaces_is_left_bare() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.env = vec![("NODE_ENV".into(), "production".into())];
        let unit = render(&spec).expect("render");
        assert!(
            unit.contains("Environment=NODE_ENV=production\n"),
            "unit was:\n{unit}"
        );
    }

    #[test]
    fn an_env_value_with_quotes_or_backslashes_is_escaped() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.env = vec![("MSG".into(), "say \"hi\" c:\\x".into())];
        let unit = render(&spec).expect("render");
        assert!(
            unit.contains(r#"Environment="MSG=say \"hi\" c:\\x""#),
            "unit was:\n{unit}"
        );
    }

    /// Percent-escaping runs before quoting, so a value needing both keeps its
    /// doubled `%%` inside the quotes rather than having the escape mangled.
    #[test]
    fn an_env_value_needing_both_escapes_gets_both() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.env = vec![("PW".into(), "50% off today".into())];
        let unit = render(&spec).expect("render");
        assert!(
            unit.contains("Environment=\"PW=50%% off today\""),
            "unit was:\n{unit}"
        );
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
