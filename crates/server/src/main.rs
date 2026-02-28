use anyhow::Result;
use axum::Router;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod auth;
mod db;
mod routes;
mod services;

pub struct AppState {
    pub db: db::Database,
    pub docker: services::docker::DockerService,
    pub traefik: services::traefik::TraefikService,
    pub config: AppConfig,
    pub metrics: services::monitor::MetricsState,
}

pub struct AppConfig {
    pub jwt_secret: String,
    pub admin_username: String,
    pub admin_password_hash: String,
    pub host: String,
    pub port: u16,
    pub data_dir: String,
    /// Host port Traefik listens on for HTTP traffic (default 80).
    pub traefik_http_port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "dockyy=info,tower_http=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    dotenvy::dotenv().ok();

    let data_dir = std::env::var("DOCKYY_DATA_DIR").unwrap_or_else(|_| "./data".into());
    std::fs::create_dir_all(&data_dir)?;

    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
        tracing::warn!("JWT_SECRET not set, using random secret (sessions will not survive restarts)");
        uuid::Uuid::new_v4().to_string()
    });

    let admin_username = std::env::var("ADMIN_USERNAME").unwrap_or_else(|_| "admin".into());
    let admin_password = std::env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "admin".into());
    let admin_password_hash = bcrypt::hash(&admin_password, 4)?; // cost=4 for speed

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".into())
        .parse()?;

    let traefik_http_port: u16 = std::env::var("TRAEFIK_HTTP_PORT")
        .unwrap_or_else(|_| "80".into())
        .parse()?;

    // Initialize database
    let db_path = format!("{}/dockyy.db", &data_dir);
    let database = db::Database::new(&db_path)?;
    database.run_migrations()?;
    tracing::info!("Database initialized at {}", db_path);

    // Initialize Docker service
    let docker = services::docker::DockerService::new().await?;
    tracing::info!("Docker client connected");

    // Initialize Traefik service (shares the Docker socket)
    let traefik = services::traefik::TraefikService::new(
        bollard::Docker::connect_with_local_defaults()?,
    );

    let config = AppConfig {
        jwt_secret,
        admin_username,
        admin_password_hash,
        host: host.clone(),
        port,
        data_dir,
        traefik_http_port,
    };

    let state = Arc::new(AppState {
        db: database,
        docker,
        traefik,
        config,
        metrics: services::monitor::new_metrics_state(),
    });

    // Ensure Traefik sidecar is running (non-fatal — log and continue)
    if let Err(e) = state.traefik.ensure_traefik(state.config.traefik_http_port).await {
        tracing::warn!("Could not start Traefik sidecar: {}", e);
    } else {
        tracing::info!(
            "Traefik reverse proxy ready on port {}",
            state.config.traefik_http_port
        );
    }

    // Spawn job worker
    tokio::spawn(services::worker::run_worker(state.clone()));

    // Spawn health monitor
    tokio::spawn(services::monitor::run_monitor(state.clone()));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .nest("/api", routes::api_routes(state.clone()))
        .fallback(routes::static_files::serve_static)
        .with_state(state)
        .layer(cors);

    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Dockyy server listening on http://{}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}
