use std::path::Path;

use anyhow::{Context, Result};
use kuadrat_core::spec::{Route, WorkloadSpec};
use kuadrat_core::store::{AppConfig, Store};

/// Back-fill `app_config` from `kuadrat deploy`'s own arguments, before the
/// daemon handoff is attempted.
///
/// H2 added `app_config` (what the daemon's API reads) alongside `apps`
/// (what a local deploy reads) but nothing wrote the former for a CLI
/// deploy — see docs/known-gaps.md, "CLI-deployed apps have no
/// registration". H7's daemon handoff made that load-bearing: the deploy
/// handler 404s on an app with no `app_config` row, and a 404 is a refusal,
/// which this CLI deliberately never falls back from. Calling this before
/// the handoff on every run, not only the first, both fixes the first
/// deploy of a brand-new app and keeps the registration converged on
/// whatever repo path and route were last passed on the command line.
///
/// `repo` must be canonicalised before it reaches `register_app`, which
/// rejects a relative `repo_path` — the daemon's working directory is not
/// this shell's, so a relative path would resolve against the wrong place.
pub fn backfill_registration(
    store: &Store,
    app: &str,
    repo: &Path,
    route: Option<Route>,
) -> Result<()> {
    let abs = repo
        .canonicalize()
        .with_context(|| format!("no such path: {}", repo.display()))?;
    let repo_path = abs
        .to_str()
        .with_context(|| format!("repo path {} is not valid UTF-8", abs.display()))?
        .to_string();
    store.register_app(&AppConfig {
        name: app.to_string(),
        repo_path,
        route,
    })
}

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
    use std::path::Path;
    use tempfile::tempdir;

    /// The whole point of C1's fix: `kuadrat deploy` writes the row the
    /// daemon's handoff would otherwise 404 on, from its own arguments,
    /// before the handoff is ever attempted.
    #[test]
    fn backfill_registration_writes_an_app_config_row() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();

        backfill_registration(&store, "web", &repo, None).unwrap();

        let config = store.app_config("web").unwrap().expect("registered");
        assert_eq!(config.name, "web");
        // Must be absolute: the daemon's working directory is not this
        // shell's, and `Store::register_app` itself rejects a relative path.
        assert!(
            Path::new(&config.repo_path).is_absolute(),
            "{}",
            config.repo_path
        );
    }

    /// Run on every deploy, not only the first, so a later deploy with a
    /// different `--route` (or none) overwrites what an earlier one wrote —
    /// the registration converges on what the operator last asked for.
    #[test]
    fn backfill_registration_converges_on_the_latest_route() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();

        let route = Route {
            domain: "example.com".into(),
            port: 3000,
        };
        backfill_registration(&store, "web", &repo, Some(route.clone())).unwrap();
        assert_eq!(store.app_config("web").unwrap().unwrap().route, Some(route));

        // The operator deploys again without --route: the registration must
        // drop it too, not keep serving the stale one.
        backfill_registration(&store, "web", &repo, None).unwrap();
        assert_eq!(store.app_config("web").unwrap().unwrap().route, None);
    }

    /// A path that does not exist must fail with a message pointing at what
    /// the operator actually typed, not `register_app`'s less specific
    /// "not absolute" — canonicalisation is what surfaces this one first.
    #[test]
    fn backfill_registration_reports_a_missing_path_clearly() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let missing = dir.path().join("does-not-exist");

        let err = backfill_registration(&store, "web", &missing, None).unwrap_err();
        assert!(err.to_string().contains("no such path"), "{err}");
    }

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
