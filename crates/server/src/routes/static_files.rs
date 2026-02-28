use axum::{
    http::{header, StatusCode, Uri},
    response::{Html, IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../dashboard/dist/"]
struct DashboardAssets;

pub async fn serve_static(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try to serve the exact file
    if let Some(file) = <DashboardAssets as Embed>::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime.as_ref())],
            file.data.to_vec(),
        )
            .into_response();
    }

    // Fallback to index.html for SPA routing
    match <DashboardAssets as Embed>::get("index.html") {
        Some(file) => Html(String::from_utf8_lossy(&file.data).to_string()).into_response(),
        None => (StatusCode::NOT_FOUND, "Dashboard not built. Run: cd dashboard && npm run build").into_response(),
    }
}
