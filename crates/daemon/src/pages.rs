//! The operator's pages.
//!
//! Rendered with `maud`, which escapes every interpolation by default. That
//! default is the whole reason it is here: these pages carry journald content,
//! which kuadrat cannot vouch for — `known-gaps.md` records it as "whatever
//! the application wrote to its stdout and stderr". `maud::PreEscaped` does
//! not appear in this module, and should not.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{sse, IntoResponse, Response};
use kuadrat_core::deploy::DeployStatus;
use kuadrat_core::events::StoredEvent;
use kuadrat_core::logs::tail;
use kuadrat_core::store::DeployRow;
use kuadrat_core::workloads::query::status;
use maud::{html, Markup, DOCTYPE};

use crate::api::summarise;
use crate::error::ApiResult;
use crate::state::AppState;
use crate::stream::events_sse;

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

/// One row of a deploy's timeline — the single renderer for a row, whether it
/// is drawn from the stored backlog or delivered as a live SSE fragment. A
/// second copy of this markup, built separately for the two call sites, is
/// how a live row and a reloaded row end up looking different, and whoever
/// hits that will blame the stream, not this function.
fn event_row(ev: &StoredEvent) -> Markup {
    let (stage, status) = ev.event.kind.columns();
    html! {
        li class="deploy-event" {
            span class="event-stage" { (stage) }
            " "
            span class="event-status" { (status) }
            @if let Some(detail) = &ev.event.detail {
                " — " (detail)
            }
        }
    }
}

/// `GET /deploy/:id`'s body, for both an in-progress and a finished deploy.
/// One function serves both because the events are durable either way — a
/// terminal deploy has its whole story already in `events`, and an
/// in-progress one has what has happened so far plus a stream for the rest.
///
/// `live` gates every htmx attribute through maud's optional-attribute
/// syntax, so a terminal deploy emits none of them at all rather than an
/// empty `sse-connect=""`. A finished deploy that still opened a stream would
/// be a connection that can only close — the reconnect loop the 204 rule in
/// `events_sse` exists to prevent, reintroduced here instead.
fn deploy_page(row: &DeployRow, events: &[StoredEvent], live: bool) -> Markup {
    let connect = live.then(|| format!("/deploy/{}/stream", row.id));
    html! {
        h1 { "Deploy " (row.id) " — " (row.app) }
        p { "Status: " (row.status.as_str()) }
        ul #timeline
            hx-ext=[live.then_some("sse")]
            sse-connect=[connect.as_deref()]
            sse-swap=[live.then_some("message")]
            hx-swap=[live.then_some("beforeend")]
        {
            @for ev in events { (event_row(ev)) }
        }
    }
}

/// `GET /deploy/:id`: a deploy's page, live or finished.
pub async fn deploy_detail(State(st): State<AppState>, Path(id): Path<i64>) -> Response {
    let row = match st.store.deploy(id) {
        Ok(Some(r)) => r,
        Ok(None) => return not_found("deploy"),
        Err(e) => return store_unavailable("the deploy", e),
    };

    let events = match st.store.events_for(id) {
        Ok(evs) => evs,
        Err(e) => return store_unavailable("the deploy's events", e),
    };

    let live = row.status == DeployStatus::InProgress;
    let title = format!("deploy {}", row.id);
    layout(&title, deploy_page(&row, &events, live)).into_response()
}

/// `GET /deploy/:id/stream`: the same `event_row` the page renders, sent as
/// an htmx SSE fragment instead of a full page. `events_sse` owns everything
/// about *when* an event reaches this handler; this closure only says what it
/// looks like once it does.
pub async fn deploy_stream(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    events_sse(&st, id, &headers, |ev| {
        sse::Event::default()
            .id(ev.id.to_string())
            .data(event_row(ev).into_string())
    })
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

#[cfg(test)]
mod tests {
    use crate::api::tests::{body_text, get, harness_parts, sse_raw_data, stage_event};
    use axum::http::StatusCode;
    use kuadrat_core::deploy::{DeployStatus, Stage};
    use kuadrat_core::events::{Event, EventSink, EventStatus};
    use tower::ServiceExt;

    /// A terminal deploy renders its whole timeline and attaches no stream —
    /// there is nothing to wait for, and an SSE connection that can only close
    /// is a reconnect loop waiting to happen.
    #[tokio::test]
    async fn a_finished_deploy_renders_its_timeline_without_a_stream() {
        let (app, store, _hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        stage_event(&store, id, Stage::Build, EventStatus::Started);
        store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        store
            .finish_deploy(id, DeployStatus::Done, None)
            .expect("finish");

        let body = body_text(
            app.oneshot(get(&format!("/deploy/{id}")))
                .await
                .expect("send"),
        )
        .await;
        assert!(
            body.contains("build"),
            "the stored timeline is missing: {body}"
        );
        assert!(
            !body.contains("sse-connect"),
            "a finished deploy must not open a stream"
        );
    }

    #[tokio::test]
    async fn an_in_progress_deploy_attaches_to_its_stream() {
        let (app, store, _hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        stage_event(&store, id, Stage::Detect, EventStatus::Started);

        let body = body_text(
            app.oneshot(get(&format!("/deploy/{id}")))
                .await
                .expect("send"),
        )
        .await;
        assert!(
            body.contains(&format!("sse-connect=\"/deploy/{id}/stream\"")),
            "no stream attached: {body}"
        );
        assert!(
            body.contains("hx-swap=\"beforeend\""),
            "rows must append, not replace"
        );
    }

    /// The stream sends the same fragment the page renders. If these diverge, a
    /// row that arrived live looks different from the same row after a reload.
    #[tokio::test]
    async fn the_stream_sends_the_same_row_markup_the_page_renders() {
        let (app, store, hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");

        let res = app
            .clone()
            .oneshot(get(&format!("/deploy/{id}/stream")))
            .await
            .expect("send");

        let live = store
            .append_event(&Event::for_stage(
                id,
                Stage::Build,
                EventStatus::Started,
                None,
            ))
            .expect("append");
        hub.emit(&live);
        let end = store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        hub.emit(&end);

        let streamed = sse_raw_data(res).await;
        assert!(streamed[0].contains("build"), "fragment: {}", streamed[0]);
        assert!(
            streamed[0].starts_with("<li"),
            "fragment must be a row: {}",
            streamed[0]
        );

        let page = body_text(
            app.oneshot(get(&format!("/deploy/{id}")))
                .await
                .expect("send"),
        )
        .await;
        assert!(
            page.contains(streamed[0].trim()),
            "the page and the stream disagree about a row's markup"
        );
    }

    #[tokio::test]
    async fn an_unknown_deploy_page_is_an_html_404() {
        let (app, _store, _hub, _d) = harness_parts();
        let res = app.oneshot(get("/deploy/999")).await.expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
