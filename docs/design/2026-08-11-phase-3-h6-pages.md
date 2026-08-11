# Phase 3 · H6 — The Pages

The three htmx pages, the embedded assets, and the browser half of the deploy action. This is the
group that turns the daemon from an API into something an operator can look at.

Parent design: [`2026-08-11-phase-3-daemon-and-surfaces.md`](./2026-08-11-phase-3-daemon-and-surfaces.md)
— §"Surfaces" (the route tables) and §"The SSE stream". That document decided *what* the pages are;
this one decides how they are built, and records four choices it left open.

## Goal

An operator opens `http://127.0.0.1:7457/`, sees every registered app and its live status, registers
a new one, clicks into it, presses redeploy, and watches the six stages arrive one by one without
touching a terminal.

## The four decisions this group makes

The parent design named htmx and embedded assets and left the rest open. Decided here:

| Decision | Choice | Why |
|---|---|---|
| Frontend library | **htmx, vendored and embedded** | The parent design's call, kept. Gives a declarative path for the actions rather than hand-written fetch code |
| HTML generation | **`maud`** — HTML as a compile-time Rust macro | Escapes by default, checked at compile time, no separate template files to drift from the handlers |
| Action surface | **Deploy *and* registration** | Registration exists because a browser has no argv; leaving it out would make the UI a viewer that still needs the CLI to be useful |
| SSE payload | **A second, page-facing stream** emitting HTML fragments | Keeps `/api/deploys/:id/events` a JSON contract for the CLI and phase 4's agent, while htmx's `sse-swap` gets the HTML it needs |

The escaping property is the reason the HTML-generation choice matters more than it looks. These
pages interpolate data kuadrat does not control: app names, domains, the error text of a failed
stage, and — most dangerous — journald content, which `known-gaps.md` records as "whatever the
application wrote to its stdout and stderr." A hand-rolled `write!` would put an escaping decision
at every interpolation site, and the one that gets missed is on the log line.

## Modules

Three new files in `crates/daemon/src/`, each with one responsibility:

| File | Responsibility |
|---|---|
| `stream.rs` | The SSE engine: subscribe-then-backlog ordering, the `id > last_sent` filter, lag recovery, termination. Renderer-agnostic |
| `pages.rs` | maud rendering and the page handlers |
| `assets.rs` | The embedded static files and their content types |

`api.rs` keeps the JSON API and loses the stream machinery to `stream.rs`.

### Splitting the stream engine

`deploy_events` currently weaves together two things of very different difficulty:

- **Ordering** — subscribe before any read, send the backlog, forward live events with
  `id > last_sent`, recover from `RecvError::Lagged` by re-reading SQLite, close when the deploy
  ends. This is the hard part. It cost a fix round during H5 and a second one after the whole-branch
  review found the `finish_deploy`/`append_event` gap.
- **Rendering** — turn a `StoredEvent` into an `sse::Event`. Three lines.

H6 needs a second stream that differs *only* in the second. Copying the first would create two
copies of the property that is hardest to verify and whose divergence would be hardest to see — and
the ordering is already untestable in one respect (no `.await` point exists between the subscribe
and the backlog read, so no test can distinguish the two orderings; the doc comment is the guard).

So `stream.rs` owns the ordering once and takes the renderer as a parameter:

```rust
pub fn events_sse<F>(st: &AppState, id: i64, resume: i64, render: F)
    -> ApiResult<Response>
where
    F: Fn(&StoredEvent) -> sse::Event + Send + 'static;
```

`api.rs` passes a renderer producing `EventOut` JSON; `pages.rs` passes one producing an `<li>`
fragment. **The eight existing JSON stream tests must pass unchanged** — that is the evidence the
extraction preserved behaviour, and it is worth more than any new test written for the refactor.

## The reconnect loop, and the 204 rule

Not covered by the parent design, and it would ship as a bug.

`EventSource` reconnects automatically when the server closes a stream. Our stream closes on the
terminal event by design. So the browser reconnects a few seconds later, the handler sees a deploy
that is already terminal, sends the backlog again or nothing, closes — and the browser reconnects
again. A finished deploy left open in a tab polls the daemon forever.

The fix is in the HTML specification already: **a `204 No Content` response stops `EventSource` from
reconnecting.** The rule both stream handlers follow:

| Condition | Response |
|---|---|
| deploy is terminal **and** the client has already seen every event (`Last-Event-ID` ≥ the last stored id) | `204`, no body |
| anything else | `200`, backlog then live, closing when the deploy ends |

The two halves matter separately. A first connection to a finished deploy still gets its whole
timeline, because the client has seen nothing. The *reconnection* after the stream closes carries a
`Last-Event-ID` at the end of the log, gets the 204, and stops. One extra round trip, then silence.

This also covers the deploy that ends with no event at all — `reserve` rejecting a duplicate — since
"has seen everything" is trivially true of an empty log.

## Pages

| Route | Renders |
|---|---|
| `GET /` | app list with current status, and the registration form |
| `GET /app/:name` | status, route, image, recent deploys, log tail, redeploy button |
| `GET /deploy/:id` | one deploy — live if in progress, static timeline if terminal |
| `GET /deploy/:id/stream` | the page-facing SSE stream: HTML fragments |

`/deploy/:id` uses one rendering function for both cases, as the parent design requires. A terminal
deploy renders its stored events and attaches no stream. An in-progress one renders what has
happened so far and attaches to `/deploy/:id/stream` with `hx-ext="sse"`, `sse-connect`, and
`hx-swap="beforeend"` into the timeline list. The events are durable either way, which is what lets
one function serve both.

The `sse-connect` URL must tell the stream where that server-rendered timeline ended —
`/deploy/:id/stream?resume=<last id already on the page>` — because the browser's first
`EventSource` connection to it carries no `Last-Event-ID` of its own. Without that hint, `events_sse`
would read `resume` as 0 and replay the whole backlog on top of what the page already rendered, and
`hx-swap="beforeend"` would append every one of those rows a second time. `Last-Event-ID` still wins
over the query parameter when a real reconnect carries one, since it reflects what the client has
actually received rather than what one page render happened to contain.

The status shown in the app list is a host read per request, as `summarise` already does today.

## Actions

`POST /api/apps/:name/deploy` gains content negotiation, as the parent design specified but H4 did
not implement: `Accept: application/json` returns `{"deploy_id": N}` exactly as it does now, so the
CLI is untouched; anything else gets `303 See Other` to `/deploy/:id`. The redeploy button is a
plain form posting to that route.

Registration gets a form-encoded handler posting to `POST /apps` — distinct from the JSON
`POST /api/apps` — that redirects to `/app/:name` on success. Keeping the form route separate from
the API route avoids one handler branching on content type for both its input and its output.

Validation failures render the form again with the error, rather than returning a bare 400. A
registration rejected for a relative `repo_path` must say so on the page the operator is looking at.

## Assets

htmx, its SSE extension, and one stylesheet, embedded with `include_str!` and served from
`/assets/*`. The daemon binds loopback on a host that may have no outbound network, so a CDN
reference would break the UI exactly where kuadrat is meant to run.

Vendored files carry provenance: a comment recording the upstream URL, the exact version, and the
SHA-256 of what was downloaded. Vendored code with no provenance cannot be audited or updated
safely, and this is the only third-party code in the repository.

Note that htmx 2 ships its SSE support as a separate extension package, so this is two files, not
one. Exact versions are pinned when they are vendored.

## Error handling

Page routes answer in HTML, not JSON — an operator who mistypes a URL should not get a JSON blob.

| Condition | Page response |
|---|---|
| unknown app or deploy | 404 page |
| status read fails | the row renders "Unknown"; the list still renders (already true of `summarise`) |
| log read fails | the page renders with a notice in the log section |
| registration rejected | the form re-renders with the reason |

The log case is the one worth stating: a single unreadable journal must not blank the whole app
detail page. The failure belongs in the section that failed.

## Testing

Handlers through `oneshot`, asserting on the rendered HTML.

The test that matters most: **an app whose log line contains `<script>` must render it as text.**
That is the least trusted data path in the system — journald content, which kuadrat cannot vouch
for — arriving in an operator's browser. `maud` escapes by default, so this test pins that nothing
later bypasses it with a raw-HTML escape hatch.

Also: the 204-when-exhausted rule, both first-connection and reconnection; asset content types; the
`303`-versus-JSON split on the deploy route; and the eight existing JSON stream tests passing
unchanged after the extraction.

## Not in H6

- **Live log tailing.** Phase 4, which has two consumers for it. `logs::tail` stays a bounded read.
- **Authentication.** The daemon is loopback-only in this phase; log content is as sensitive as the
  least careful app on the host, which is why it binds where it does.
- **Pagination.** Recent deploys are bounded by a fixed limit, not paged.
- **The webhook sender and `kuadrat serve` wiring.** H7.

## Fixed quantities

Named here rather than left to the implementer, so two people reading this build the same page:

- `/app/:name` shows the **10** most recent deploys, newest first.
- It tails **100** log lines — the same default the JSON logs endpoint already uses, so the page and
  the API agree about what "the recent log" means.

## Deliberately not doing

**The app list does not refresh itself.** htmx makes it one attribute, and it is tempting. But the
list is what an operator reads while deciding what to do, and content that moves under a reader is
worse than content that is a few seconds stale. A deploy in progress has its own page, which does
update live. Revisit only if using it proves otherwise.

## Open questions

None. The two quantities above were the only ones, and they are fixed.
