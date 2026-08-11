//! The SSE engine: everything about *when* an event reaches a subscriber, and
//! nothing about what it looks like when it gets there.
//!
//! Two handlers need this: the JSON API stream and the page's HTML stream.
//! They differ only in how one event is rendered, which is three lines. What
//! they share — subscribe before any read, send the backlog, forward live
//! events with `id > last_sent`, recover from a lag by re-reading SQLite,
//! close when the deploy ends — is the part that is hard to get right, cost a
//! fix round in H5 and another after the whole-branch review, and cannot be
//! fully covered by tests: no `.await` point exists between the subscribe and
//! the backlog read, so no test can tell the correct ordering from the
//! reversed one. A second copy of that would be a second place for it to rot
//! silently.

use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{self, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use kuadrat_core::deploy::DeployStatus;
use kuadrat_core::events::{EventKind, StoredEvent};
use tokio::sync::broadcast::error::RecvError;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// One deploy's events as an SSE response, rendered by `render`.
///
/// **Subscribing happens before every read**, the deploy row included. An
/// event landing between the backlog read and the subscription would be lost
/// permanently, and it is precisely the stage transition the viewer is waiting
/// for; in this order it arrives twice instead, and the `id > last_sent`
/// filter drops the duplicate. A duplicate is recoverable; a gap is not. The
/// row read carries no ordering requirement of its own and sits behind the
/// subscription only so that nobody has to reason about which reads count.
///
/// `render` returns the payload, not a finished `sse::Event` — building the
/// event is this function's job, not the renderer's. `sse::Event::data`
/// panics if its argument contains a `\r` (`axum`'s SSE writer asserts no
/// field value does; the wire format is line-based, and a bare CR breaks its
/// framing) and event details are `format!("{err:#}")` of a stage failure,
/// which can embed raw command stderr — `podman build` in particular routinely
/// emits `\r`. Parameterising the renderer moved that constraint out of the
/// engine and into whichever closure happened to satisfy it by accident (JSON
/// escaping does; string interpolation does not); sanitising here, once, makes
/// it structural instead — true of every renderer, including the next one.
/// Likewise the id: it comes from the store, not from `render`, so the
/// `Last-Event-ID` resumption contract cannot be broken by a renderer that
/// forgets to set it.
pub fn events_sse<F>(st: &AppState, id: i64, headers: &HeaderMap, render: F) -> ApiResult<Response>
where
    F: Fn(&StoredEvent) -> String + Send + 'static,
{
    // Subscribing has no side effect and costs nothing, so it happens before
    // every other step here — including the 404 check below.
    let mut rx = st.hub.subscribe();

    // 404 before a stream is opened. An unknown id must fail as a request —
    // a stream that opens and then says nothing is indistinguishable, to a
    // browser, from a deploy that is simply slow.
    let row = st
        .store
        .deploy(id)
        .map_err(|e| ApiError::internal(format!("reading deploy {id}: {e:#}")))?
        .ok_or_else(|| ApiError::not_found(format!("no deploy {id}")))?;

    let backlog = st
        .store
        .events_for(id)
        .map_err(|e| ApiError::internal(format!("reading events for {id}: {e:#}")))?;

    let already_terminal = row.status != DeployStatus::InProgress;

    // The recovery path re-reads SQLite from inside the stream, which outlives
    // the borrow of `st`.
    let store = st.store.clone();

    // Clamped to the highest id this deploy has actually stored. `resume` is
    // untrusted client input seeded straight into `last_sent`; an id above
    // anything the store has ever assigned this deploy is impossible for a
    // real client to have seen (ids are only handed out on insert), so
    // trusting it verbatim would filter out every backlog and live event,
    // including the terminal one, and the stream would never yield anything
    // or close. Clamping treats that case the same as no resume point at
    // all past what is already known, rather than as a promise to skip
    // events that have not happened yet.
    let last_id = backlog.last().map_or(0, |ev| ev.id);
    let resume = resume_from(headers).min(last_id);

    // A finished deploy with nothing the client has not already seen. Closing
    // a stream is how this handler says "the deploy ended" — but `EventSource`
    // reads a closed stream as a dropped connection and reconnects a few
    // seconds later, forever. `204 No Content` is the response the HTML
    // specification defines as "do not reconnect", so a finished deploy left
    // open in a tab goes quiet after exactly one extra round trip.
    //
    // Both halves of the condition earn their place. Without `already_terminal`
    // a live deploy that a viewer is caught up with would be told to go away
    // mid-run. Without the seen-everything half, the *first* connection to a
    // finished deploy would get a 204 and render an empty timeline instead of
    // its history.
    if already_terminal && resume >= last_id {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    let stream = async_stream::stream! {
        let mut last_sent = resume;
        let mut ended = false;

        for ev in backlog {
            if ev.id <= resume {
                continue;
            }
            last_sent = ev.id;
            ended = is_finished(&ev);
            yield Ok::<_, std::convert::Infallible>(to_sse_event(ev.id, render(&ev)));
        }

        // Nothing more can arrive for a deploy that has already ended. The
        // second half of the condition covers a deploy finished by a path that
        // emits no event — `reserve` rejecting a duplicate leaves a terminal
        // row with an empty log — where waiting for a terminal event would
        // mean waiting forever.
        if ended || already_terminal {
            return;
        }

        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if ev.event.deploy_id != id || ev.id <= last_sent {
                        continue;
                    }
                    last_sent = ev.id;
                    let ends_here = is_finished(&ev);
                    yield Ok::<_, std::convert::Infallible>(to_sse_event(ev.id, render(&ev)));
                    if ends_here {
                        return;
                    }
                }
                // The slow-receiver case, and the failure mode the design
                // singles out as most likely to be got wrong. Treating it as
                // fatal closes a viewer's stream mid-deploy; ignoring it
                // silently skips stages. The dropped events are still in
                // SQLite — they were persisted before they were published — so
                // re-read from the last id sent and resume. This is the same
                // path a reconnection takes.
                //
                // `events_for` re-reads every event for the deploy rather than
                // querying `WHERE id > last_sent`: a deploy has on the order of
                // thirteen events total, so the wasted rows are noise, and this
                // is a deliberate choice at that scale, not an oversight — a
                // deploy with orders of magnitude more events would want the
                // narrower query.
                Err(RecvError::Lagged(_)) => {
                    let missed = match store.events_for(id) {
                        Ok(evs) => evs,
                        // The store is unreadable; ending the stream is the
                        // honest answer, and the browser will reconnect.
                        Err(_) => return,
                    };
                    for ev in missed {
                        if ev.id <= last_sent {
                            continue;
                        }
                        last_sent = ev.id;
                        let ends_here = is_finished(&ev);
                        yield Ok::<_, std::convert::Infallible>(to_sse_event(ev.id, render(&ev)));
                        if ends_here {
                            return;
                        }
                    }
                }
                // Every sender is gone: the daemon is shutting down.
                Err(RecvError::Closed) => return,
            }
        }
    };

    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

fn is_finished(ev: &StoredEvent) -> bool {
    matches!(ev.event.kind, EventKind::Finished { .. })
}

/// Build the wire `sse::Event` for one stored event from a renderer's
/// payload. The id always comes from the store, never from `render` — see
/// `events_sse`'s doc comment for why that and the `\r`/`\n` sanitisation both
/// live here instead of in each renderer.
fn to_sse_event(id: i64, data: String) -> sse::Event {
    let sanitized = data.replace(['\r', '\n'], " ");
    sse::Event::default().id(id.to_string()).data(sanitized)
}

/// The id a reconnecting browser last saw. `EventSource` sets this header
/// itself from the `id:` field of the last event it received, so honouring it
/// is what makes a dropped connection resume rather than replay.
///
/// A value that is not a number is treated as absent. It is a hint, not a
/// command: failing the request would break the reconnect it exists to serve,
/// while replaying from the start is always correct and merely chattier.
pub fn resume_from(headers: &HeaderMap) -> i64 {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0)
}
