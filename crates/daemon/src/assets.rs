//! The embedded UI assets.
//!
//! `include_str!` rather than a runtime read: the binary is the deployment
//! unit, and an asset that can be missing at runtime is a page that breaks on
//! a host nobody can debug from. See `assets/PROVENANCE.md` for the origin
//! and hash of the two vendored files.

use axum::http::header;
use axum::response::IntoResponse;

const HTMX: &str = include_str!("../assets/htmx.min.js");
const SSE: &str = include_str!("../assets/sse.min.js");
const CSS: &str = include_str!("../assets/kuadrat.css");

const JS: &str = "text/javascript; charset=utf-8";
const CSS_TYPE: &str = "text/css; charset=utf-8";

pub async fn htmx() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, JS)], HTMX)
}

pub async fn sse() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, JS)], SSE)
}

pub async fn css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, CSS_TYPE)], CSS)
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use tower::ServiceExt;

    use crate::api::tests::get;

    #[tokio::test]
    async fn htmx_is_served_as_javascript() {
        let (app, _store, _hub, _d) = crate::api::tests::harness_parts();
        let res = app.oneshot(get("/assets/htmx.min.js")).await.expect("send");
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/javascript; charset=utf-8")
        );
    }

    #[tokio::test]
    async fn the_stylesheet_is_served_as_css() {
        let (app, _store, _hub, _d) = crate::api::tests::harness_parts();
        let res = app.oneshot(get("/assets/kuadrat.css")).await.expect("send");
        assert_eq!(
            res.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/css; charset=utf-8")
        );
    }

    /// A wrong content type is the failure mode here: a browser will not execute a
    /// script served as `text/plain`, and the page fails in a way that looks like
    /// htmx is broken rather than like the server is.
    #[tokio::test]
    async fn the_sse_extension_is_served_and_is_not_empty() {
        let (app, _store, _hub, _d) = crate::api::tests::harness_parts();
        let res = app.oneshot(get("/assets/sse.min.js")).await.expect("send");
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("body");
        assert!(
            body.len() > 1000,
            "sse extension looks truncated: {} bytes",
            body.len()
        );
    }
}
