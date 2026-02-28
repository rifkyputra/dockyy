use axum::Router;
use std::sync::Arc;

use crate::AppState;

pub mod auth;
pub mod containers;
pub mod health;
pub mod repositories;
pub mod static_files;
pub mod deployments;
pub mod webhooks;

pub fn api_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let public_routes = Router::new()
        .merge(health::routes())
        .merge(auth::routes())
        .merge(webhooks::routes());

    let protected_routes = Router::new()
        .merge(containers::routes())
        .merge(repositories::routes())
        .merge(deployments::routes())
        .merge(health::metrics_routes())
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::auth_middleware,
        ));

    Router::new()
        .merge(public_routes)
        .merge(protected_routes)
}
