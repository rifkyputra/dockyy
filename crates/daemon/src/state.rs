//! Shared handler state, and how a registered app becomes a deployable spec.

use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use kuadrat_core::events::EventSink;
use kuadrat_core::exec::Executor;
use kuadrat_core::fs::FileSystem;
use kuadrat_core::spec::WorkloadSpec;
use kuadrat_core::store::{AppConfig, Store};
use kuadrat_core::workloads::paths::Paths;
use tokio::sync::Semaphore;

/// Everything a handler needs. Cloned per request; every field is an `Arc` or
/// a cheap value, so cloning costs nothing.
#[derive(Clone)]
pub struct AppState {
    pub exec: Arc<dyn Executor>,
    pub fsys: Arc<dyn FileSystem>,
    pub sink: Arc<dyn EventSink>,
    pub store: Arc<Store>,
    pub paths: Paths,
    /// One permit, globally: one deploy at a time.
    ///
    /// RAM is the binding constraint on these hosts and `podman build` is the
    /// spikiest consumer. This is a *resource policy*, not a correctness
    /// mechanism — the store's per-app lock is the correctness backstop, and
    /// removing this semaphore must not make anything unsafe.
    pub deploy_slot: Arc<Semaphore>,
}

impl AppState {
    pub fn new(
        exec: Arc<dyn Executor>,
        fsys: Arc<dyn FileSystem>,
        sink: Arc<dyn EventSink>,
        store: Arc<Store>,
        paths: Paths,
    ) -> Self {
        Self {
            exec,
            fsys,
            sink,
            store,
            paths,
            deploy_slot: Arc::new(Semaphore::new(1)),
        }
    }
}

/// Build the spec to deploy for a registered app.
///
/// Sources, in order: a `kuadrat.json` in the registered repo, else the spec
/// stored from the app's last deploy. The name is forced to the registered
/// name.
///
/// **The route comes from the registration, unconditionally — including when it
/// is `None`.** `app_config` is the operator's intent and is authoritative for
/// `repo_path` and `route`; `apps.spec_json` is the deploy record and is
/// authoritative for `image` and the resolved spec.
///
/// This deliberately does *not* reuse the CLI's `resolve_spec(.., route_override:
/// Option<Route>)`, where `None` means "don't override" rather than "no route".
/// Passing a registration's `None` through that parameter would make clearing a
/// route in the UI silently ineffective: the operator removes the domain, the
/// next deploy re-applies the old route from the stored spec, and the Caddy
/// fragment goes straight back up.
pub fn spec_for(config: &AppConfig, fsys_read: impl FnOnce(&Path) -> Option<String>) -> Result<WorkloadSpec> {
    let repo = Path::new(&config.repo_path);
    let file = repo.join("kuadrat.json");

    let mut spec: WorkloadSpec = match fsys_read(&file) {
        Some(text) => serde_json::from_str(&text)
            .with_context(|| format!("parsing {}", file.display()))?,
        None => bail!(
            "no spec for {}: add a kuadrat.json to {} or deploy it once with one",
            config.name,
            repo.display()
        ),
    };

    spec.name = config.name.clone();
    spec.route = config.route.clone();
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuadrat_core::spec::Route;

    fn config(route: Option<Route>) -> AppConfig {
        AppConfig {
            name: "web".into(),
            repo_path: "/srv/web".into(),
            route,
        }
    }

    fn spec_json(route: Option<Route>) -> String {
        let mut spec = WorkloadSpec::new("whatever", "old-image");
        spec.ports = vec!["3000:3000".into()];
        spec.health_cmd = Some("true".into());
        spec.route = route;
        serde_json::to_string(&spec).unwrap()
    }

    fn route() -> Route {
        Route {
            domain: "example.com".into(),
            port: 3000,
        }
    }

    #[test]
    fn the_repo_spec_supplies_everything_but_name_and_route() {
        let spec = spec_for(&config(None), |_| Some(spec_json(None))).expect("resolve");
        assert_eq!(spec.ports, vec!["3000:3000".to_string()]);
    }

    #[test]
    fn the_name_is_forced_to_the_registered_name() {
        let spec = spec_for(&config(None), |_| Some(spec_json(None))).expect("resolve");
        assert_eq!(spec.name, "web");
    }

    #[test]
    fn the_registrations_route_wins_over_the_repo_spec() {
        let stale = Route {
            domain: "old.example.com".into(),
            port: 9999,
        };
        let spec = spec_for(&config(Some(route())), |_| Some(spec_json(Some(stale))))
            .expect("resolve");
        assert_eq!(spec.route, Some(route()));
    }

    #[test]
    fn a_registration_without_a_route_clears_the_repo_specs_route() {
        // The authority rule's whole point. If this ever reads "don't
        // override", clearing a domain in the UI stops working and the next
        // deploy puts the Caddy fragment back.
        let spec = spec_for(&config(None), |_| Some(spec_json(Some(route())))).expect("resolve");
        assert_eq!(spec.route, None);
    }

    #[test]
    fn a_missing_spec_is_an_error_naming_the_repo() {
        let err = spec_for(&config(None), |_| None).expect_err("no spec");
        assert!(err.to_string().contains("/srv/web"), "was: {err}");
    }
}
