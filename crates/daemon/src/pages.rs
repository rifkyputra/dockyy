//! The operator's pages.
//!
//! Rendered with `maud`, which escapes every interpolation by default. That
//! default is the whole reason it is here: these pages carry journald content,
//! which kuadrat cannot vouch for — `known-gaps.md` records it as "whatever
//! the application wrote to its stdout and stderr". `maud::PreEscaped` does
//! not appear in this module, and should not.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use maud::{html, Markup, DOCTYPE};

use crate::api::summarise;
use crate::state::AppState;

/// The shell every page shares.
fn layout(title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "kuadrat — " (title) }
                link rel="stylesheet" href="/assets/kuadrat.css";
                script src="/assets/htmx.min.js" {}
                script src="/assets/sse.min.js" {}
            }
            body {
                header { a href="/" { "kuadrat" } }
                main { (body) }
            }
        }
    }
}

/// The app list at `GET /`.
///
/// A store read that fails renders an empty list rather than a 500: this page
/// is where an operator goes when something is wrong, and failing closed here
/// — a blank page instead of the list they came to see — is worse than
/// rendering thin with what little is known.
pub async fn index(State(st): State<AppState>) -> Markup {
    let configs = st.store.list_app_configs().unwrap_or_default();

    let body = html! {
        h1 { "Apps" }
        @if configs.is_empty() {
            p { "No apps registered yet." }
        } @else {
            table id="apps" {
                thead {
                    tr {
                        th { "Name" }
                        th { "Repo" }
                        th { "Route" }
                        th { "Status" }
                    }
                }
                tbody {
                    @for config in configs {
                        @let summary = summarise(&st, config).await;
                        tr {
                            td { a href={ "/app/" (summary.name) } { (summary.name) } }
                            td { (summary.repo_path) }
                            td {
                                @if let Some(route) = &summary.route {
                                    (route.domain)
                                } @else {
                                    "—"
                                }
                            }
                            td { (summary.status) }
                        }
                    }
                }
            }
        }
    };

    layout("apps", body)
}

/// A plain HTML 404, for page routes — kept apart from the JSON API's error
/// shape so a browser navigating to a missing page gets a page back, not a
/// JSON blob.
pub fn not_found(what: &str) -> Response {
    let body = html! {
        h1 { "Not found" }
        p { "No such " (what) "." }
    };
    (StatusCode::NOT_FOUND, layout("not found", body)).into_response()
}
