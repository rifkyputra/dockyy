# kuadrat phase 3 — the daemon and its surfaces

- **Date:** 2026-08-11
- **Status:** approved design, not yet implemented
- **Builds on:** [phase 1 design](2026-08-10-design.md) and [phase 2 design](2026-08-10-phase-2-deploy-loop.md),
  which shipped `spec`, `exec`, `fs`, `workloads`, `store`, `events`, `gateway`, `secrets` and the
  deploy state machine, all verified on a real host

## Goal

A deploy can be run and watched from a browser.

Phase 2 built the loop. Phase 3 puts a networked surface in front of it and adds nothing to what
kuadrat can *do* — every capability already exists in `core`. This phase is about who can reach it
and what they see while it runs.

## Scope

### In

- **Daemon** — `kuadrat serve`: axum HTTP API, three htmx pages, SSE, one systemd unit
- **Live events** — an `EventSink` seam in `core`, broadcast to SSE subscribers
- **Logs** — a `logs` module: bounded journald reads scoped to a unit
- **Outbound webhook** — a POST on terminal outcomes and stage failures
- **App registration** — persisting a repo path so an app can be redeployed without argv

### Out

- **Authentication, sessions, TLS of kuadrat's own** — the daemon binds loopback and a unix socket;
  reaching it is the operator's job (phase 1 decision, unchanged)
- **Live log tailing** (`journalctl -f`) — deferred to phase 4, which needs it for agent diagnosis
- **MCP** — phase 4
- **Push-to-deploy / webhook receiver** — v1 non-goal; deploys stay explicitly triggered
- **Multi-user anything** — no accounts, no roles, no audit-by-user

## Decisions

| Decision | Choice | Why |
|---|---|---|
| Who runs a deploy | The daemon, exclusively | One writer, one event bus; every deploy is visible regardless of trigger |
| Progress reporting | An `EventSink` seam on `Ctx` | Matches the two existing seams; keeps transport out of `core` |
| Binary layout | One binary, `kuadrat serve` | One file to install and no client/server skew on a host |
| Code layout | A `crates/daemon` library crate | Keeps axum out of the CLI's dependency list; gives the web code its own test target |
| Logs | Bounded `tail`/`search` only | Phase 3's criterion is carried by events; streaming lands in phase 4 with two consumers |
| Webhook scope | Terminal outcomes and stage failures | The receiver wants warnings, not a trace; ~1–3 POSTs per deploy |
| Webhook guarantee | Best-effort, bounded retry | The events table is the record; the webhook is a doorbell |
| UI shape | Three pages, incl. `/deploy/:id` | A deploy gets an addressable URL that phase 4's agent can point at |
| Concurrency | One deploy at a time, globally | RAM is the binding constraint on these hosts, and `podman build` is the spikiest consumer |

## Architecture

```
browser ── HTTP/SSE ──┐
kuadrat deploy ───────┤   loopback :7457  +  unix socket
                      ▼
        crates/daemon  — the only networked code
          router · handlers · SSE hub · templates · webhook sender
                      │  direct calls, no socket
                      ▼
        crates/core    — three seams, no transport
          Executor · FileSystem · EventSink
                      │
        crates/cli     — the kuadrat binary; `serve` calls daemon::serve()
```

Crate dependencies point one way: `cli → daemon → core`. The daemon never imports the cli, and
`core` imports neither. A fleet driver later becomes a fourth consumer of `core` without touching
the daemon.

### The third seam

```rust
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &StoredEvent);
}
```

`emit` returns nothing and is synchronous. A subscriber that has gone away must not be able to fail
a deploy, so there is no error to propagate; and a sink with no way to await cannot block the deploy
loop on I/O. The signature carries both guarantees instead of a comment asking for them. A sink
needing async work — the H7 webhook — subscribes to the broadcast channel and does that work in its
own task.

Events reach a sink as `StoredEvent { id, event }` rather than as an `Event` carrying its own id.
An id only exists after the insert, so a type that carries one cannot be built before persisting:
persist-before-publish becomes a property of the types rather than a rule to remember.

`Ctx` gains a fifth field, `sink`, alongside `exec`, `fsys`, `store`, and `paths`. `deploy::run`
funnels every transition through one helper that calls `store.append_event(&event)?` and publishes
the `StoredEvent` it returns. **Persist first, then publish**: the durable record is what the API
serves on reconnect, so it must never lag the stream. This ordering is load-bearing, not stylistic
— the lag-recovery path in the SSE handler depends on it, and the `?` means a failed insert
publishes nothing.

Three implementations:

| Impl | Where | Behaviour |
|---|---|---|
| `BroadcastSink` | daemon | wraps `tokio::sync::broadcast::Sender<StoredEvent>` |
| `NullSink` | cli | drops everything; used by the in-process `apply`/`remove` paths |
| `FakeSink` | tests | records to a `Vec` for assertions |

**ADR-0002 needs a fourth clause** naming `EventSink` alongside `Executor` and `FileSystem`, the way
G1 added the store carve-out. This adds a seam; it does not pierce one. `core` still has no HTTP
dependency and no `host` parameter.

### What changes in `core`

- `events`: `StoredEvent { id, at, event }`, the `EventSink` trait, `NullSink`, `FakeSink`.
  `store::append_event` returns `Result<StoredEvent>` — `INSERT … RETURNING id, at`, so the
  id and the insert timestamp come back from the same statement — and `events_for` returns
  `Vec<StoredEvent>`. **Both columns already exist** (`events.id INTEGER PRIMARY KEY AUTOINCREMENT`,
  `events.at TEXT NOT NULL DEFAULT (datetime('now'))`) — this is an API exposure change, not a
  schema change.
- `store`: a new `app_config` table — `name`, `repo_path`, `route_domain`, `route_port` — with
  `register_app`, `app_config` and `list_app_configs` accessors. **Not** new columns on `apps`: see
  Registration storage below.
- `logs`: a new module, `tail(exec, unit, n)` and `search(exec, unit, pattern)` over the existing
  `Executor`.
- `deploy::run` and `reconcile`: emit through the sink after each persisted event.

Nothing else in `core` moves.

### Registration storage

Registration does **not** extend the `apps` table, and there is no `ALTER TABLE`.

`apps` is `name TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, spec_json TEXT NOT NULL`. A
registration exists *before* an app's first deploy — that is its purpose, since a browser has no
argv to supply a repo path from — so at registration time there is no spec and no slug, and the row
cannot be inserted at all. Nullable new columns do not help: the blocker is a `NOT NULL` on an
existing column, and SQLite cannot drop one without rebuilding the table that holds user data.

A separate table sidesteps it entirely:

```sql
CREATE TABLE IF NOT EXISTS app_config (
    name         TEXT PRIMARY KEY,
    repo_path    TEXT NOT NULL,
    route_domain TEXT,
    route_port   INTEGER,
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
```

Added to the existing `SCHEMA` batch, this is correct on an existing database by construction —
`IF NOT EXISTS` creates it where it is missing and does nothing where it is not. No migration step,
no idempotency question.

The split is also honest about meaning: `apps` records **what was deployed**, written by the deploy
loop on success; `app_config` records **what the operator asked for**, written by registration. They
have different lifetimes and different writers.

The route is two nullable columns rather than one blob so a query can filter on domain without
parsing. They are written together and read together — one without the other means the row was
edited outside kuadrat, and the read refuses it rather than serving half a route. The upsert writes
both columns unconditionally, including when the route is `None`: an upsert that skipped nulls would
make clearing a route impossible, leaving an app served on a domain the operator had just removed.

**Authority rule.** `app_config` and `apps.spec_json` can each carry a `route`, and nothing so far
has said which one wins on a deploy. The rule: `app_config` is the operator's intent and is
authoritative for `repo_path` and `route`; `apps` is the deploy record and is authoritative for
`image` and the resolved spec. When an `app_config` row exists, a deploy must assign
`spec.route = config.route` unconditionally — including `None`. **Do not** wire this through
`resolve_spec(app, repo, store, route_override: Option<Route>)` in `crates/cli/src/resolve.rs` by
passing `config.route` as `route_override` — in `resolve_spec`, `None` means "don't override" (keep
whatever route the repo's `kuadrat.json` or the stored spec already carries), not "no route". That
reading is the obvious one for the next group to reach for, and it is exactly wrong here: this group
made clearing a route work on purpose (an unconditional upsert, and the test
`re_registering_without_a_route_clears_the_previous_one`), and routing through `route_override` would
make that silently ineffective — the operator clears a route in the UI, the next deploy re-applies
the old one from the stored spec, and the Caddy fragment goes back up.

One consequence worth flagging for the next group: `WorkloadSpec::validate` (`crates/core/src/spec.rs`)
rejects a spec that has a `route` but no `health_cmd`. So a UI "add a domain" action can fail a
deploy for a reason that, from the UI's point of view, looks unrelated to the domain field it just
changed — the fix is in `health_cmd`, not in the route.

**`repo_path` validation.** `register_app` rejects a relative `repo_path`: the daemon runs under
systemd with `/` as its working directory, not the operator's shell, so a relative path resolves
against the wrong place. It deliberately does **not** check for `..` traversal or symlinks in the
path — the operator who registers an app already has that filesystem access by the time they can
reach the registration API (loopback-only, no auth in this phase), so checking here would not move
the trust boundary, only add a check that looks like a security control without being one.

## Surfaces

### Pages

| Route | Renders |
|---|---|
| `GET /` | app list with current status |
| `GET /app/:name` | detail: status, route, image, recent deploys, log tail |
| `GET /deploy/:id` | one deploy — live if in progress, static timeline if terminal |

`/deploy/:id` uses one template for both cases. A terminal deploy renders its stored events and
sends no stream; an in-progress one renders what has happened so far and attaches to the stream for
the rest. The events are durable either way, which is what lets a single template serve both.

Assets — htmx itself and the stylesheet — are **embedded in the binary**. The daemon binds loopback
on a host that may have no outbound network, so a CDN reference would leave the UI broken exactly
where kuadrat is meant to run.

### API

| Route | Purpose |
|---|---|
| `GET /api/apps` | list with status |
| `GET /api/apps/:name` | one app |
| `POST /api/apps` | register: name, repo path, optional route |
| `POST /api/apps/:name/deploy` | trigger; returns `{deploy_id}` |
| `GET /api/deploys/:id` | row + events |
| `GET /api/deploys/:id/events` | SSE |
| `GET /api/apps/:name/logs?n=` | bounded journald read |

Content negotiation splits page from API: the deploy handler returns `303 See Other` to
`/deploy/:id` for a browser and `{"deploy_id": N}` for `Accept: application/json`. The CLI's
`kuadrat deploy` is a client of the JSON form.

### Registration, and why it is needed

`deploy::run` takes a repo path as an argument and nothing persists it — the CLI reads it from argv
on every run. A browser has no argv, so "redeploy" from the UI has nowhere to get the source.
`POST /api/apps` writes `repo_path` and `route`; every later deploy of that app reads them. Without
this the UI could only deploy apps it was told about in the same request, which is not a UI.

`POST /api/apps/:name/deploy` therefore takes no body for a redeploy.

### The SSE stream

```
subscribe to the broadcast channel   ← first, before any read
read stored events from SQLite
send them as the backlog
forward live events with id > last sent
```

Subscribing **before** reading closes the join gap. An event landing between the read and the
subscribe would otherwise be lost permanently, and it is precisely the stage transition the viewer
wants. This ordering converts a lost event into a duplicate, which the id filter drops.

Resumption is free once each event is a `StoredEvent` carrying its id: a browser reconnecting sends
`Last-Event-ID` and the handler replays from there.

The stream closes when the deploy reaches a terminal status.

## Error handling

**A failed deploy is not an HTTP error.** `POST /api/apps/:name/deploy` succeeds as soon as the
deploy is *accepted*; whether it worked arrives over the stream. This mirrors the contract
`deploy::run` already has, where "could not begin" is `Err` and "ran and rolled back" is
`Ok(RolledBack)`.

| Condition | Response |
|---|---|
| unknown app | `404` |
| a deploy of that app is already in progress | `409` |
| spec fails `validate()`, or a registration is missing a repo path | `400` |
| accepted | `303` (browser) / `200 {deploy_id}` (JSON) |

**Broadcast lag is the failure mode most likely to be gotten wrong.**
`tokio::sync::broadcast` drops messages for a slow receiver and reports `RecvError::Lagged`.
Treating it as fatal closes the viewer's stream mid-deploy; ignoring it silently skips stages.
Correct handling is to re-read from SQLite starting at the last id sent, then resume live — the same
path as reconnection. This works only because events are persisted before they are published.

**Webhook delivery never affects a deploy.** Failures are logged and dropped after the bounded
retry. The sink is `emit`-and-forget by type, so this is enforced rather than remembered.

**Startup runs `deploy::reconcile` to completion before binding.** A crashed deploy is rolled back
while nothing can observe half-state, and the first page load shows a settled system.

## Concurrency

One deploy at a time, globally: a `tokio::sync::Semaphore` with one permit, held by the daemon. The
per-app lock in the store stays as the correctness backstop — the semaphore is a resource policy,
not a correctness mechanism, and removing it must not make anything unsafe.

**Two different mechanisms, two different answers**, and the distinction has to be visible in the
handler:

| Situation | Mechanism | Result |
|---|---|---|
| a deploy of *this* app is in progress | per-app lock | `409`, immediately |
| a deploy of *another* app is in progress | global semaphore | accepted, queued |

The in-progress check therefore runs **before** the request waits on the semaphore. Reversed, a
duplicate deploy of a busy app would sit in the queue for minutes only to be rejected on reaching
the front — the rejection is knowable at once and must be returned at once.

A queued deploy holds an allocated `deploy_id` and a row while it waits, so `/deploy/:id` can render
"queued" immediately after the redirect. That row is `in_progress`, which means a crash while queued
leaves a row that G5's `reconcile` will roll back on next start. This is harmless — nothing has
happened yet to undo — and is recorded here so it does not later read as a bug.

## Configuration

```
kuadrat serve --listen 127.0.0.1:7457 \
              --socket /run/kuadrat/kuadrat.sock \
              --root /etc/containers/systemd
```

**The webhook URL is deliberately not a flag.** A Telegram or Slack webhook URL carries its token in
the path, and argv is world-readable via `ps` — the same reasoning that put secret values on stdin
in G3. It is read from `KUADRAT_WEBHOOK_URL` or a file at startup, is never logged in full, and
appears in error messages as the host only.

**Exposure is enforced in code.** If `--listen` resolves to a non-loopback address the daemon
**refuses to start**, with an error naming SSH tunnelling and VPN as the intended path. There is no
login, no session handling and no TLS of kuadrat's own. This is the phase 1 decision, unchanged: a
self-rolled auth stack on a root-privileged daemon is a larger risk than not exposing the port.

## Testing

| Level | Approach |
|---|---|
| `core` | `FakeSink` asserts a full deploy emits Started/Succeeded per stage in order, and that a failing stage emits Failed followed by the compensation events. `logs` scripts `journalctl` output through `FakeExecutor`. |
| `daemon` | Handlers tested in-process with `tower::ServiceExt::oneshot` against a `Ctx` built from `FakeExecutor` + `FakeFileSystem` + `FakeSink` + a temp-file `Store`. No socket bound, no podman required. |
| real host | `scripts/serve-acceptance.sh`, matching the five existing scripts. |

The SSE tests cover the three cases that matter:

1. backlog then live, in order and without a gap
2. an event delivered both ways at the join is sent once
3. after a simulated `Lagged`, the stream recovers from the store and misses nothing

The acceptance script starts the daemon, registers and deploys a real repo over the socket, and
asserts the stream carried six stages, the pages render, and the app answers. It must also assert
the daemon **refuses** a non-loopback `--listen`, since that guard is a security boundary and an
untested guard is an assumed one.

## Task groups

| Group | Content |
|---|---|
| **H1** | `EventSink` seam, the three impls, `StoredEvent`, sink calls in `run` (`reconcile` deferred — see known-gaps), ADR-0002 fourth clause |
| **H2** | `app_config` table, `register_app`/`app_config`/`list_app_configs`, the idempotency tests |
| **H3** | `logs` module — `tail` and `search` |
| **H4** | `crates/daemon`: config, loopback guard, router, JSON API, the global semaphore and the before-queue 409, reconcile-then-bind |
| **H5** | SSE: the deploy-level terminal event, broadcast hub, backlog-then-live, dedupe, lag recovery, `Last-Event-ID` |
| **H6** | The three htmx pages and embedded assets |
| **H7** | Webhook sender; `kuadrat serve` wiring, the systemd unit, `kuadrat deploy` as an API client, `serve-acceptance.sh` |

H1–H3 are `core` work and independent of each other, so they can land in any order. H4 depends on H2
(registration) and H3 (the logs endpoint); H5 depends on H1 and H4; H6 depends on H5; H7 closes the
phase.

## Open questions

- Whether the systemd unit uses socket activation. It would let the daemon idle at zero RAM until a
  request arrives, which suits the low-memory premise, but it complicates the reconcile-before-bind
  ordering. Decide during H7, when the unit is written.
- Whether `kuadrat status`/`list` should prefer the daemon when it is running rather than always
  reading the host directly. Both answers are defensible; not needed for phase 3's criterion.
