//! The daemon: the only networked code in kuadrat.
//!
//! It adds nothing to what kuadrat can do — every capability already exists in
//! `core`. This crate decides who can reach those capabilities and what they
//! see while a deploy runs. Dependencies point one way, `cli → daemon → core`:
//! the daemon never imports the cli, and `core` imports neither, so a fleet
//! driver later becomes another consumer of `core` without touching this.

pub mod api;
pub mod assets;
pub mod config;
pub mod error;
pub mod hooks;
pub mod hub;
pub mod pages;
pub mod state;
pub mod stream;
pub mod webhook;

use std::sync::Arc;

use anyhow::{Context, Result};
use kuadrat_core::deploy::reconcile;
use kuadrat_core::exec::local::LocalExecutor;
use kuadrat_core::fs::local::LocalFileSystem;
use kuadrat_core::store::Store;
use kuadrat_core::workloads::paths::Paths;

pub use config::Config;
pub use state::AppState;

/// Start the daemon: guard, recover, then bind.
///
/// The ordering is deliberate. `reconcile` runs to completion **before** the
/// listener binds, so a deploy left in flight by a crash is rolled back while
/// nothing can observe the half-state, and the first page load shows a settled
/// system rather than a stage that will never advance.
pub async fn serve(config: Config) -> Result<()> {
    config.validate()?;

    let paths = match &config.root {
        Some(root) => Paths::rooted(root),
        None => Paths::default(),
    };
    let store = Arc::new(Store::open(&paths.db_path)?);

    let mut state = AppState::new(
        Arc::new(LocalExecutor),
        Arc::new(LocalFileSystem),
        store,
        paths,
    );
    state.hook_secret = hooks::HookSecret::from_env()
        .context("loading inbound hook secret")?
        .map(Arc::new);

    let recovered = reconcile(&state.ctx())
        .await
        .context("reconciling crashed deploys at startup")?;
    for outcome in &recovered {
        eprintln!("reconciled: {outcome:?}");
    }

    match webhook::Webhook::from_env() {
        Ok(Some(hook)) => webhook::spawn(&state, hook),
        Ok(None) => {} // Not configured. Not an error, and not worth a line on every start.
        Err(e) => eprintln!("webhook disabled: {e:#}"),
    }

    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("binding {}", config.listen))?;
    eprintln!("kuadrat listening on http://{}", config.listen);

    axum::serve(listener, api::router(state))
        .await
        .context("serving")
}
