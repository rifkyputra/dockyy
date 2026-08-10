use std::path::Path;

use anyhow::{Context, Result};
use kuadrat_core::spec::{Route, WorkloadSpec};
use kuadrat_core::store::Store;

/// Resolve one `WorkloadSpec` for `app` from, in order: a `kuadrat.json` in the
/// repo, else the spec stored from a prior deploy. The name is forced to `app`,
/// and `route_override` (a CLI flag) replaces any route in the resolved spec.
pub fn resolve_spec(
    app: &str,
    repo: &Path,
    store: &Store,
    route_override: Option<Route>,
) -> Result<WorkloadSpec> {
    let file = repo.join("kuadrat.json");
    let mut spec: WorkloadSpec = if file.exists() {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", file.display()))?
    } else if let Some(json) = store.current_spec(app)? {
        serde_json::from_str(&json).context("parsing the stored spec")?
    } else {
        anyhow::bail!(
            "no spec for {app}: add a kuadrat.json to {} or deploy it once with one",
            repo.display()
        );
    };

    spec.name = app.to_string();
    if let Some(route) = route_override {
        spec.route = Some(route);
    }
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuadrat_core::spec::{Route, WorkloadSpec};
    use kuadrat_core::store::Store;
    use tempfile::tempdir;

    #[test]
    fn a_repo_kuadrat_json_is_the_primary_source() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("kuadrat.json"),
            r#"{"name":"ignored","image":"","command":null,"env":[],"ports":["3000:3000"],"volumes":[],"secrets":[],"memory_max":null,"health_cmd":null,"restart_policy":"Always","route":null}"#,
        )
        .unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        let spec = resolve_spec("web", dir.path(), &store, None).unwrap();
        assert_eq!(spec.name, "web"); // name forced to the app arg
        assert_eq!(spec.ports, vec!["3000:3000".to_string()]);
    }

    #[test]
    fn the_stored_spec_is_the_fallback_when_no_file() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let mut prior = WorkloadSpec::new("web", "old");
        prior.ports = vec!["8080:8080".into()];
        store
            .put_spec("web", "web", &serde_json::to_string(&prior).unwrap())
            .unwrap();

        let spec = resolve_spec("web", dir.path(), &store, None).unwrap();
        assert_eq!(spec.ports, vec!["8080:8080".to_string()]);
    }

    #[test]
    fn no_file_and_no_stored_spec_is_an_error() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let err = resolve_spec("web", dir.path(), &store, None).unwrap_err();
        assert!(err.to_string().contains("kuadrat.json"), "message: {err}");
    }

    #[test]
    fn a_route_override_wins() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("kuadrat.json"),
            r#"{"name":"web","image":"","command":null,"env":[],"ports":[],"volumes":[],"secrets":[],"memory_max":null,"health_cmd":"true","restart_policy":"Always","route":null}"#,
        )
        .unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        let route = Route {
            domain: "example.com".into(),
            port: 3000,
        };
        let spec = resolve_spec("web", dir.path(), &store, Some(route.clone())).unwrap();
        assert_eq!(spec.route, Some(route));
    }
}
