//! The operator's pages.
//!
//! Rendered with `maud`, which escapes every interpolation by default. That
//! default is the whole reason it is here: these pages carry journald content,
//! which kuadrat cannot vouch for — `known-gaps.md` records it as "whatever
//! the application wrote to its stdout and stderr". `maud::PreEscaped` does
//! not appear in this module, and should not.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use kuadrat_core::logs::tail;
use kuadrat_core::workloads::query::status;
use maud::{html, Markup, DOCTYPE};

use crate::api::summarise;
use crate::state::AppState;

/// Deploys shown on an app's page. Fixed by the design document rather than
/// left to taste, so the page and anyone reading the spec agree.
const RECENT_DEPLOYS: usize = 10;

/// Log lines tailed on an app's page — the same default the JSON logs endpoint
/// uses, so the page and the API mean the same thing by "the recent log".
const LOG_LINES: usize = 100;

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

/// An app's detail page at `GET /app/:name`: status, route, image, its
/// `RECENT_DEPLOYS` most recent deploys, and a `LOG_LINES`-line log tail.
///
/// A registration that genuinely does not exist and one this handler failed
/// to read are kept apart on purpose: they send the operator in different
/// directions. "No such app" says check the spelling, or re-register it. A
/// store read failure says check the disk, or the database — `index`'s
/// fail-thin bias does not carry over here, because there the two cases both
/// mean "nothing to click into right now" and here they do not.
pub async fn app_detail(State(st): State<AppState>, Path(name): Path<String>) -> Response {
    let config = match st.store.app_config(&name) {
        Ok(Some(c)) => c,
        Ok(None) => return not_found("app"),
        Err(e) => return store_unavailable("the registration", e),
    };

    let status_label = match status(&*st.exec, &*st.fsys, &st.paths, &name).await {
        Ok(s) => s.label(),
        Err(_) => "Unknown",
    };

    // `current_spec` is `None` for an app that has never deployed — that is
    // not an error, just nothing to show yet.
    let image = st
        .store
        .current_spec(&name)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|v| v.get("image")?.as_str().map(str::to_string));

    let deploys = st
        .store
        .recent_deploys(&name, RECENT_DEPLOYS)
        .unwrap_or_default();

    // The interesting part: one unreadable journal renders a note, not a
    // blank page. This is the one failure mode this handler must not let
    // escape as a 500 or an empty body.
    let log_section = match tail(&*st.exec, &name, LOG_LINES).await {
        Ok(lines) if lines.is_empty() => html! {
            p id="log-empty" { "No output yet." }
        },
        Ok(lines) => html! {
            pre id="log-tail" { (lines.join("\n")) }
        },
        Err(e) => html! {
            p id="log-error" { "Could not read the journal: " (format!("{e:#}")) }
        },
    };

    let body = html! {
        h1 { (config.name) }
        dl id="app-facts" {
            dt { "Status" }
            dd { (status_label) }
            dt { "Repo" }
            dd { (config.repo_path) }
            dt { "Route" }
            dd {
                @if let Some(route) = &config.route {
                    (route.domain) ":" (route.port)
                } @else {
                    "—"
                }
            }
            dt { "Image" }
            dd {
                @if let Some(image) = &image {
                    (image)
                } @else {
                    "—"
                }
            }
        }

        h2 { "Recent deploys" }
        @if deploys.is_empty() {
            p { "No deploys yet." }
        } @else {
            table id="deploy-history" {
                thead {
                    tr {
                        th { "Deploy" }
                        th { "Stage" }
                        th { "Status" }
                        th { "Detail" }
                    }
                }
                tbody {
                    @for d in &deploys {
                        tr {
                            td { a href={ "/deploy/" (d.id) } { (d.id) } }
                            td { (d.stage.as_str()) }
                            td { (d.status.as_str()) }
                            td {
                                @if let Some(detail) = &d.detail {
                                    (detail)
                                } @else {
                                    "—"
                                }
                            }
                        }
                    }
                }
            }
        }

        h2 { "Log" }
        (log_section)
    };

    layout(&config.name, body).into_response()
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

/// A store read that failed outright, distinct from `not_found`: this is not
/// "no such app", it is "could not find out". Named after what could not be
/// read so the operator knows what to go check, the same shape the log
/// section already uses for an unreadable journal.
fn store_unavailable(what: &str, e: anyhow::Error) -> Response {
    let body = html! {
        h1 { "Could not read " (what) }
        p id="store-error" { "The store could not be read: " (format!("{e:#}")) }
    };
    (StatusCode::INTERNAL_SERVER_ERROR, layout("error", body)).into_response()
}
