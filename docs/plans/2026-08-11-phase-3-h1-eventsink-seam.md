# kuadrat Phase 3 · H1 — The EventSink Seam Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deploy events can leave `core` while a deploy is still running, so the daemon can stream
them to a browser.

**Architecture:** A third seam beside `Executor` and `FileSystem`. `EventSink::emit` publishes an
event that has already been persisted; the store assigns the id, so publishing without persisting is
impossible by construction. Two implementations ship in `core` (`NullSink`, `FakeSink`); the
broadcast-backed one arrives with the daemon in H5.

**Tech Stack:** Rust 2021, `anyhow`, `rusqlite` (bundled), `tokio` (test runtime only in this
group), `std::sync::Mutex`.

**Design:** [`docs/design/2026-08-11-phase-3-daemon-and-surfaces.md`](../design/2026-08-11-phase-3-daemon-and-surfaces.md)

## Global Constraints

- **`core` never opens a socket and never takes a `host` parameter.** Adding a seam is allowed;
  piercing one is not. No HTTP or transport dependency enters `crates/core` in this group.
- **`make check` must pass**: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
  A formatting slip fails the build — run `cargo fmt` before every commit.
- **`cargo test --all` must pass.** Baseline is **123 tests** at `8172b22`; the count only goes up.
- **No new dependencies.** Everything here uses what the workspace already declares.
- **Secret values never appear** in events, logs, error messages, or committed files. An event's
  `detail` carries an error string — never a secret value.
- Commit after every task with a Conventional Commit subject.

## Two corrections to the design document

The design was written before the call sites were read. Both changes are deliberate; Task 4 amends
the design so the document and the code agree.

**1. `emit` is synchronous, not `async`.** The design specified `#[async_trait] async fn emit`. The
three emission sites live in `begin`, `ok` and `fail` (`deploy/run.rs:160-199`), and `begin`/`ok` are
**sync** functions. An async `emit` would force both async, rippling `.await` through the driver for
no benefit — `tokio::sync::broadcast::Sender::send` is itself sync and non-blocking. A sync `emit`
also makes the guarantee structural: a sink *cannot* block a deploy on I/O, because it has no way to
await anything. A sink that needs to do async work (the H7 webhook) subscribes to the broadcast
channel and does that work in its own task.

**2. `StoredEvent { id, event }` rather than adding `id` to `Event`.** The design said `Event` gains
`id: i64`. It cannot, sensibly: at construction time in `begin` the id does not exist yet — SQLite
assigns it on insert. Adding the field would force every construction site to invent a placeholder
id, and a placeholder that reaches a subscriber is a bug that looks like data. Splitting the write
type from the read type makes persist-before-publish **structural**: you cannot build a
`StoredEvent` without an id, and the only source of an id is `append_event`.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/src/events/mod.rs` | *Modify.* Keeps `Event`, `EventStatus`; gains `StoredEvent` and the `EventSink` trait; declares the two submodules |
| `crates/core/src/events/null.rs` | *Create.* `NullSink` — drops everything |
| `crates/core/src/events/fake.rs` | *Create.* `FakeSink` — records for assertions |
| `crates/core/src/store/mod.rs` | *Modify.* `append_event` returns the id; `events_for` returns `Vec<StoredEvent>` |
| `crates/core/src/deploy/mod.rs` | *Modify.* `Ctx` gains a `sink` field |
| `crates/core/src/deploy/run.rs` | *Modify.* `begin`/`ok`/`fail` emit after persisting; 9 test `Ctx` sites updated |
| `crates/cli/src/main.rs` | *Modify.* 2 `Ctx` sites pass `&NullSink` |
| `docs/adr/0002-transport-agnostic-core.md` | *Modify.* Fourth clause naming `EventSink` |
| `docs/design/2026-08-11-phase-3-daemon-and-surfaces.md` | *Modify.* Reconcile the two corrections above |

The `mod.rs` / `null.rs` / `fake.rs` split follows the existing shape of `exec/` (`mod.rs`,
`local.rs`, `fake.rs`) and `fs/`. Do not invent a different layout.

---

### Task 1: The `StoredEvent` type, the `EventSink` trait, and its two implementations

**Files:**
- Modify: `crates/core/src/events/mod.rs`
- Create: `crates/core/src/events/null.rs`
- Create: `crates/core/src/events/fake.rs`

**Interfaces:**
- Consumes: `Event`, `EventStatus` (already in `events/mod.rs`), `Stage` (from `crate::deploy`)
- Produces:
  - `pub struct StoredEvent { pub id: i64, pub event: Event }`
  - `pub trait EventSink: Send + Sync { fn emit(&self, event: &StoredEvent); }`
  - `pub struct NullSink;`
  - `pub struct FakeSink` with `new()`, `events() -> Vec<StoredEvent>`,
    `timeline() -> Vec<(Stage, EventStatus)>`

- [ ] **Step 1: Write the failing tests**

Add to the existing `mod tests` block at the bottom of `crates/core/src/events/mod.rs`. Keep the
tests already there — do not replace the file.

```rust
    use crate::events::fake::FakeSink;
    use crate::events::null::NullSink;

    fn stored(id: i64, stage: Stage, status: EventStatus) -> StoredEvent {
        StoredEvent {
            id,
            event: Event {
                deploy_id: 1,
                stage,
                status,
                detail: None,
            },
        }
    }

    #[test]
    fn fake_sink_records_every_event_in_order() {
        let sink = FakeSink::new();
        sink.emit(&stored(1, Stage::Build, EventStatus::Started));
        sink.emit(&stored(2, Stage::Build, EventStatus::Succeeded));

        let seen = sink.events();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].id, 1);
        assert_eq!(seen[1].event.status, EventStatus::Succeeded);
    }

    /// The shape most deploy tests assert on: which stages happened, and how
    /// each ended. Spelling that out per-test would bury the assertion.
    #[test]
    fn fake_sink_timeline_is_stage_status_pairs() {
        let sink = FakeSink::new();
        sink.emit(&stored(1, Stage::Detect, EventStatus::Started));
        sink.emit(&stored(2, Stage::Detect, EventStatus::Succeeded));
        sink.emit(&stored(3, Stage::Build, EventStatus::Failed));

        assert_eq!(
            sink.timeline(),
            vec![
                (Stage::Detect, EventStatus::Started),
                (Stage::Detect, EventStatus::Succeeded),
                (Stage::Build, EventStatus::Failed),
            ]
        );
    }

    #[test]
    fn null_sink_accepts_events_and_does_nothing() {
        let sink = NullSink;
        sink.emit(&stored(1, Stage::Apply, EventStatus::Started));
        // No panic, no state. The assertion is that this compiles and runs:
        // NullSink is what the CLI passes, and it must never fail a deploy.
    }

    /// Both implementations must be usable behind `&dyn EventSink`, because
    /// `Ctx` holds one that way.
    #[test]
    fn both_sinks_are_usable_as_trait_objects() {
        let fake = FakeSink::new();
        let sinks: Vec<&dyn EventSink> = vec![&fake, &NullSink];
        for sink in sinks {
            sink.emit(&stored(9, Stage::Route, EventStatus::Started));
        }
        assert_eq!(fake.events().len(), 1);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --all events`
Expected: FAIL to **compile** — `unresolved import crate::events::fake`, `cannot find type
StoredEvent`.

- [ ] **Step 3: Add `StoredEvent`, the trait, and the module declarations**

In `crates/core/src/events/mod.rs`, add the submodule declarations at the **top** of the file (below
the `//!` doc comment, above `use crate::deploy::Stage;`):

```rust
pub mod fake;
pub mod null;
```

Then add, immediately after the existing `Event` struct definition:

```rust
/// An event as it exists after the store has written it: the same event, plus
/// the id SQLite assigned.
///
/// The split from [`Event`] is deliberate. An id only exists after the insert,
/// so a type that carries one cannot be constructed before persisting — which
/// makes "persist first, then publish" a property of the types rather than a
/// rule someone has to remember. The SSE stream needs the id to deduplicate at
/// the join and to honour `Last-Event-ID`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub id: i64,
    pub event: Event,
}

/// The third seam, beside [`crate::exec::Executor`] and [`crate::fs::FileSystem`].
///
/// Publishes an already-persisted event to whoever is watching. Implementations:
/// `NullSink` (drops), `FakeSink` (records, for tests), and a broadcast-backed
/// sink in the daemon.
///
/// `emit` returns nothing and is synchronous, both deliberately. A subscriber
/// that has gone away must not be able to fail a deploy, so there is no error
/// to propagate; and a sink with no way to await cannot block the deploy loop
/// on I/O. A sink needing async work should hand off to a channel and do that
/// work elsewhere.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &StoredEvent);
}
```

- [ ] **Step 4: Create `NullSink`**

Create `crates/core/src/events/null.rs`:

```rust
use crate::events::{EventSink, StoredEvent};

/// Discards every event. Used by the CLI's in-process paths, where nothing is
/// watching and the durable record in the store is the whole story.
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: &StoredEvent) {}
}
```

- [ ] **Step 5: Create `FakeSink`**

Create `crates/core/src/events/fake.rs`:

```rust
use std::sync::Mutex;

use crate::deploy::Stage;
use crate::events::{EventSink, EventStatus, StoredEvent};

/// Records every emitted event so tests can assert on the timeline.
#[derive(Default)]
pub struct FakeSink {
    events: Mutex<Vec<StoredEvent>>,
}

impl FakeSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every event emitted so far, in order.
    pub fn events(&self) -> Vec<StoredEvent> {
        self.events.lock().expect("sink lock").clone()
    }

    /// `(stage, status)` pairs in order — the shape most deploy tests assert
    /// on, where the ids and details are noise.
    pub fn timeline(&self) -> Vec<(Stage, EventStatus)> {
        self.events()
            .iter()
            .map(|e| (e.event.stage, e.event.status))
            .collect()
    }
}

impl EventSink for FakeSink {
    fn emit(&self, event: &StoredEvent) {
        self.events.lock().expect("sink lock").push(event.clone());
    }
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --all events`
Expected: PASS — the four new tests plus the two that were already there.

- [ ] **Step 7: Run the full gate**

Run: `cargo fmt && cargo test --all && cargo clippy --all-targets -- -D warnings`
Expected: all pass; total test count **127**.

If clippy flags `new_without_default` on `FakeSink::new`, it is wrong here — `Default` **is**
derived. If it flags anything else, fix the code rather than allowing the lint.

- [ ] **Step 8: Commit**

```bash
git add crates/core/src/events/
git commit -m "feat(core): add the EventSink seam with null and fake sinks

The third seam beside Executor and FileSystem. StoredEvent carries the id
the store assigned, so an event cannot be published before it is
persisted — the type enforces the ordering the SSE stream depends on.

emit is sync and returns nothing: a subscriber that has gone away must
not be able to fail a deploy, and a sink with no way to await cannot
block the deploy loop."
```

---

### Task 2: The store assigns and returns event ids

**Files:**
- Modify: `crates/core/src/store/mod.rs` (`append_event` ~line 233, `events_for` ~line 249,
  `event_row` ~line 302, and three test call sites at ~523, ~545, ~546)

**Interfaces:**
- Consumes: `StoredEvent` from Task 1
- Produces:
  - `Store::append_event(&self, event: &Event) -> Result<i64>` — returns the assigned id
  - `Store::events_for(&self, deploy_id: i64) -> Result<Vec<StoredEvent>>`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block at the bottom of `crates/core/src/store/mod.rs`:

```rust
    #[test]
    fn append_event_returns_the_assigned_id() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let id = store.create_deploy("web").unwrap();

        let first = store
            .append_event(&Event {
                deploy_id: id,
                stage: Stage::Build,
                status: EventStatus::Started,
                detail: None,
            })
            .expect("append");
        let second = store
            .append_event(&Event {
                deploy_id: id,
                stage: Stage::Build,
                status: EventStatus::Succeeded,
                detail: None,
            })
            .expect("append");

        assert!(first > 0, "id must be a real rowid, got {first}");
        assert!(
            second > first,
            "ids must increase: {second} came after {first}"
        );
    }

    /// The ids the stream replays from must be the ids the store handed out,
    /// or a reconnecting browser resumes from the wrong place.
    #[test]
    fn events_for_returns_the_same_ids_append_returned() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let id = store.create_deploy("web").unwrap();

        let mut appended = Vec::new();
        for status in [EventStatus::Started, EventStatus::Succeeded] {
            appended.push(
                store
                    .append_event(&Event {
                        deploy_id: id,
                        stage: Stage::Apply,
                        status,
                        detail: None,
                    })
                    .expect("append"),
            );
        }

        let read: Vec<i64> = store
            .events_for(id)
            .expect("read")
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(read, appended);
    }

    /// Ids are unique across deploys, not per-deploy — the SSE handler filters
    /// on `id > last_sent` and would drop events if two deploys reused ids.
    #[test]
    fn event_ids_are_unique_across_deploys() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let a = store.create_deploy("a").unwrap();
        let b = store.create_deploy("b").unwrap();

        let ev = |deploy_id| Event {
            deploy_id,
            stage: Stage::Detect,
            status: EventStatus::Started,
            detail: None,
        };
        let first = store.append_event(&ev(a)).expect("append");
        let second = store.append_event(&ev(b)).expect("append");

        assert_ne!(first, second);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --all store`
Expected: FAIL to compile — `append_event` returns `()`, so `first > 0` is a type error, and
`events_for(...)` items have no `.id`.

- [ ] **Step 3: Change `append_event` to return the id**

In `crates/core/src/store/mod.rs`, replace the body of `append_event`:

```rust
    /// Append one event and return the id the store assigned it. That id is
    /// what the SSE stream deduplicates and resumes on, so it must come from
    /// the same insert rather than being counted by the caller.
    pub fn append_event(&self, event: &Event) -> Result<i64> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO events (deploy_id, stage, status, detail) VALUES (?1, ?2, ?3, ?4)",
            params![
                event.deploy_id,
                event.stage.as_str(),
                event.status.as_str(),
                event.detail
            ],
        )
        .context("appending event")?;
        Ok(conn.last_insert_rowid())
    }
```

`last_insert_rowid()` is read while the same `conn` lock is still held, so a concurrent insert
cannot come between the write and the read.

- [ ] **Step 4: Change `events_for` to return `StoredEvent`**

Replace the body of `events_for`, adding `id` as the first selected column:

```rust
    /// All events for a deploy, in insertion order, each with its id.
    pub fn events_for(&self, deploy_id: i64) -> Result<Vec<StoredEvent>> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT id, deploy_id, stage, status, detail FROM events
                 WHERE deploy_id = ?1 ORDER BY id",
            )
            .context("preparing events query")?;
        let rows = stmt
            .query_map(params![deploy_id], event_row)
            .context("querying events")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading event row")??);
        }
        Ok(out)
    }
```

- [ ] **Step 5: Update the row mapper**

Replace `event_row` and `build_event` (~line 302):

```rust
fn event_row(row: &rusqlite::Row) -> rusqlite::Result<Result<StoredEvent>> {
    Ok(build_event(
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn build_event(
    id: i64,
    deploy_id: i64,
    stage_s: String,
    status_s: String,
    detail: Option<String>,
) -> Result<StoredEvent> {
    let stage = Stage::from_str(&stage_s)
        .ok_or_else(|| anyhow!("event for deploy {deploy_id} has unknown stage {stage_s:?}"))?;
    let status = EventStatus::from_str(&status_s)
        .ok_or_else(|| anyhow!("event for deploy {deploy_id} has unknown status {status_s:?}"))?;
    Ok(StoredEvent {
        id,
        event: Event {
            deploy_id,
            stage,
            status,
            detail,
        },
    })
}
```

Update the `use` at the top of the file to bring in the new type — find the existing
`use crate::events::{Event, EventStatus};` and make it:

```rust
use crate::events::{Event, EventStatus, StoredEvent};
```

- [ ] **Step 6: Fix the three existing test call sites**

At ~line 523 the test reads events and asserts on their contents. The items are now `StoredEvent`,
so field access goes through `.event`. Find the assertions that follow
`let events = store.events_for(id).expect("read");` and prefix each event field access with
`.event` — for example `events[0].stage` becomes `events[0].event.stage`, and
`events[0].detail.as_deref()` becomes `events[0].event.detail.as_deref()`.

The two `.len()` assertions at ~545-546 need no change; `Vec::len` is unaffected by the item type.

Do not change what those tests assert. If an assertion no longer holds, that is a real regression —
stop and report it rather than editing the expectation.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --all store`
Expected: PASS, including the three new tests.

- [ ] **Step 8: Run the full gate**

Run: `cargo fmt && cargo test --all && cargo clippy --all-targets -- -D warnings`
Expected: all pass; total **130**.

- [ ] **Step 9: Commit**

```bash
git add crates/core/src/store/mod.rs
git commit -m "feat(core): the store assigns and returns event ids

append_event returns the assigned rowid, read under the same connection
lock as the insert, and events_for returns StoredEvent so a reader sees
the same ids. The SSE stream deduplicates at the backlog/live join and
resumes on Last-Event-ID; both need ids that come from the insert rather
than being counted by a caller.

The events.id column already existed — this exposes it."
```

---

### Task 3: `Ctx` carries a sink, and the deploy driver emits through it

**Files:**
- Modify: `crates/core/src/deploy/mod.rs` (the `Ctx` struct, ~line 99)
- Modify: `crates/core/src/deploy/run.rs` (`begin` ~160, `ok` ~172, `fail` ~194; 9 test `Ctx`
  literals at ~332, 371, 445, 483, 517, 538, 582, 618, 643)
- Modify: `crates/cli/src/main.rs` (2 `Ctx` literals at ~130, ~172)

**Interfaces:**
- Consumes: `EventSink`, `StoredEvent`, `FakeSink`, `NullSink` (Task 1); `append_event -> i64`
  (Task 2)
- Produces: `Ctx` with a fourth field `pub sink: &'a dyn EventSink`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/core/src/deploy/run.rs`. These assert on the sink, which no
existing test does.

```rust
    /// A deploy that reaches Done must have emitted Started and Succeeded for
    /// all six stages, in order. This is what the browser renders.
    #[tokio::test]
    async fn a_successful_deploy_emits_every_stage_in_order() {
        let h = harness_ok();
        let sink = FakeSink::new();
        let ctx = h.ctx(&sink);

        let outcome = run(
            &ctx,
            WorkloadSpec::new("web", "placeholder"),
            Path::new("/repo"),
        )
        .await
        .expect("run");
        assert!(matches!(outcome, DeployOutcome::Done { .. }), "{outcome:?}");

        use EventStatus::{Started, Succeeded};
        assert_eq!(
            sink.timeline(),
            vec![
                (Stage::Detect, Started),
                (Stage::Detect, Succeeded),
                (Stage::Build, Started),
                (Stage::Build, Succeeded),
                (Stage::Secrets, Started),
                (Stage::Secrets, Succeeded),
                (Stage::Apply, Started),
                (Stage::Apply, Succeeded),
                (Stage::Route, Started),
                (Stage::Route, Succeeded),
                (Stage::Healthcheck, Started),
                (Stage::Healthcheck, Succeeded),
            ]
        );
    }

    /// Every emitted event must carry the id the store assigned it, and the
    /// ids must ascend. A subscriber filters on `id > last_seen`; a zero or a
    /// repeat would silently drop events.
    #[tokio::test]
    async fn emitted_events_carry_ascending_store_ids() {
        let h = harness_ok();
        let sink = FakeSink::new();
        let ctx = h.ctx(&sink);

        run(
            &ctx,
            WorkloadSpec::new("web", "placeholder"),
            Path::new("/repo"),
        )
        .await
        .expect("run");

        let ids: Vec<i64> = sink.events().iter().map(|e| e.id).collect();
        assert!(!ids.is_empty(), "no events emitted");
        assert!(ids.iter().all(|&i| i > 0), "ids must be real rowids: {ids:?}");
        assert!(
            ids.windows(2).all(|w| w[1] > w[0]),
            "ids must ascend: {ids:?}"
        );
    }

    /// The last thing a watcher sees before a rollback is the Failed event for
    /// the stage that broke — that is what the UI highlights and what the
    /// webhook forwards.
    #[tokio::test]
    async fn a_failed_stage_emits_a_failed_event_naming_that_stage() {
        let h = harness_apply_fails();
        let sink = FakeSink::new();
        let ctx = h.ctx(&sink);

        let outcome = run(
            &ctx,
            WorkloadSpec::new("web", "placeholder"),
            Path::new("/repo"),
        )
        .await
        .expect("terminal outcome");
        assert!(
            matches!(outcome, DeployOutcome::RolledBack { .. }),
            "{outcome:?}"
        );

        let last = sink.events().last().cloned().expect("at least one event");
        assert_eq!(last.event.stage, Stage::Apply);
        assert_eq!(last.event.status, EventStatus::Failed);
        assert!(
            last.event.detail.is_some(),
            "a Failed event must carry its cause"
        );
    }

    /// Persist-before-publish: everything the sink saw is also in the store,
    /// with the same ids. A reconnecting browser reads the store for the
    /// backlog, so a gap here is an event the user can never recover.
    #[tokio::test]
    async fn every_emitted_event_is_also_durable_with_the_same_id() {
        let h = harness_ok();
        let sink = FakeSink::new();
        let ctx = h.ctx(&sink);

        run(
            &ctx,
            WorkloadSpec::new("web", "placeholder"),
            Path::new("/repo"),
        )
        .await
        .expect("run");

        let emitted: Vec<i64> = sink.events().iter().map(|e| e.id).collect();
        let deploy_id = emitted_deploy_id(&sink);
        let stored: Vec<i64> = ctx
            .store
            .events_for(deploy_id)
            .expect("read")
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(emitted, stored);
    }

    fn emitted_deploy_id(sink: &FakeSink) -> i64 {
        sink.events()
            .first()
            .expect("at least one event")
            .event
            .deploy_id
    }
```

**Note on `harness_ok()` and `harness_apply_fails()`:** these are added in Step 5, with full code.
They wrap the setup the existing tests already repeat inline — `tempdir`, `Store::open`,
`Paths::rooted`, `fsys_with_repo()`, `FakeExecutor::new()`, `script_clean(...)` — using the
`script_clean`, `fsys_with_repo` and `out` helpers **already defined** in this test module. The
failure case is an **Apply** failure rather than a healthcheck failure specifically because
`script_clean(&exec, sha, slug, out(1, "", "boom"))` plus a scripted `systemctl stop` is a proven
sequence in this file; inventing a new one risks the fakes rejecting a call the driver makes, which
fails the test for the wrong reason.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --all deploy`
Expected: FAIL to compile — `Ctx` has no `sink` field, and `harness_ok` does not exist.

- [ ] **Step 3: Add the `sink` field to `Ctx`**

In `crates/core/src/deploy/mod.rs`, extend the struct and its doc comment:

```rust
/// Everything a deploy stage needs, bundled so stages take one argument.
///
/// The three seams — `exec` for processes, `fsys` for storage, `sink` for
/// publishing — plus the store, which is kuadrat's own state rather than a
/// side effect on the host (see ADR-0002).
pub struct Ctx<'a> {
    pub exec: &'a dyn Executor,
    pub fsys: &'a dyn FileSystem,
    pub store: &'a Store,
    pub paths: &'a Paths,
    pub sink: &'a dyn EventSink,
}
```

Add the import beside the existing ones near the top of the same file:

```rust
use crate::events::EventSink;
```

- [ ] **Step 4: Emit from the three helpers**

In `crates/core/src/deploy/run.rs`, replace `begin`, `ok`, and the opening of `fail`. Each persists
first, then publishes with the id it got back.

```rust
/// Advance the durable stage and emit a Started event.
fn begin(ctx: &Ctx<'_>, deploy_id: i64, stage: Stage) -> Result<()> {
    ctx.store.advance_stage(deploy_id, stage)?;
    emit(ctx, deploy_id, stage, EventStatus::Started, None)
}

/// Emit a Succeeded event for a stage.
fn ok(ctx: &Ctx<'_>, deploy_id: i64, stage: Stage) -> Result<()> {
    emit(ctx, deploy_id, stage, EventStatus::Succeeded, None)
}

/// Persist one event, then publish it to the sink.
///
/// The order is load-bearing: the store is what a reconnecting subscriber
/// reads for the backlog, so an event must be durable before anyone can see
/// it. Publishing first would let a browser render a stage that a crash then
/// erases.
fn emit(
    ctx: &Ctx<'_>,
    deploy_id: i64,
    stage: Stage,
    status: EventStatus,
    detail: Option<String>,
) -> Result<()> {
    let event = Event {
        deploy_id,
        stage,
        status,
        detail,
    };
    let id = ctx.store.append_event(&event)?;
    ctx.sink.emit(&StoredEvent { id, event });
    Ok(())
}
```

In `fail`, replace the `ctx.store.append_event(&Event { ... })?;` block (~line 194) with:

```rust
    emit(
        ctx,
        deploy_id,
        stage,
        EventStatus::Failed,
        Some(cause.clone()),
    )?;
```

Update the `use` line at the top of `run.rs` from
`use crate::events::{Event, EventStatus};` to:

```rust
use crate::events::{Event, EventStatus, StoredEvent};
```

- [ ] **Step 5: Add the test harness and update the nine existing `Ctx` literals**

Inside `mod tests` in `run.rs`, add a harness that owns the fakes and hands out a `Ctx`. The
existing tests each build these inline; this replaces that repetition and gives the new tests
somewhere to get a `Ctx` with a sink.

```rust
    /// Owns the fakes so a `Ctx` can borrow them, and keeps the `TempDir`
    /// alive — dropping it would delete the database mid-test.
    struct Harness {
        exec: FakeExecutor,
        fsys: FakeFileSystem,
        store: Store,
        paths: Paths,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        /// `start_result` is what `systemctl start` returns, which is the knob
        /// that decides whether Apply succeeds.
        fn new(start_result: CommandOutput) -> Self {
            let dir = tempdir().unwrap();
            let store = Store::open(&dir.path().join("k.db")).unwrap();
            let paths = Paths::rooted(dir.path());
            let fsys = fsys_with_repo();
            let exec = FakeExecutor::new();
            script_clean(&exec, "abc123", "web", start_result);
            Self {
                exec,
                fsys,
                store,
                paths,
                _dir: dir,
            }
        }

        fn ctx<'a>(&'a self, sink: &'a dyn EventSink) -> Ctx<'a> {
            Ctx {
                exec: &self.exec,
                fsys: &self.fsys,
                store: &self.store,
                paths: &self.paths,
                sink,
            }
        }
    }

    /// Every stage succeeds; the deploy reaches Done.
    fn harness_ok() -> Harness {
        Harness::new(out(0, "", ""))
    }

    /// `systemctl start` fails, so Apply fails and compensation removes the
    /// unit — the same sequence
    /// `a_first_deploy_failing_at_apply_rolls_back_by_removing_the_unit`
    /// already proves. The second `daemon-reload` that `remove` runs is
    /// already scripted by `script_clean`.
    fn harness_apply_fails() -> Harness {
        let h = Harness::new(out(1, "", "boom"));
        h.exec
            .expect_call("systemctl", &["stop", "kuadrat-web"], out(0, "", ""));
        h
    }
```

`CommandOutput` needs importing into the test module if it is not already there:
`use crate::exec::CommandOutput;`

Then update each of the nine existing `Ctx { exec: ..., fsys: ..., store: ..., paths: ... }`
literals by adding one field. The smallest correct change is `sink: &NullSink,` — those tests assert
on the store and the executor, not on events, so a recording sink would be unused.

```rust
        let ctx = Ctx {
            exec: &exec,
            fsys: &fsys,
            store: &store,
            paths: &paths,
            sink: &NullSink,
        };
```

Add to the test module's imports:

```rust
    use crate::events::fake::FakeSink;
    use crate::events::null::NullSink;
    use crate::events::EventSink;
```

**Leave the nine existing tests otherwise untouched.** They keep their own inline setup; only the
one added `sink:` field changes. Rewriting them onto the harness is a larger diff that would hide
whether this task changed behaviour, and behaviour is exactly what must not change here.

- [ ] **Step 6: Update the two CLI call sites**

In `crates/cli/src/main.rs`, both `Ctx` literals (~130, ~172) gain the same field. The CLI has no
subscribers — the store is the record — so it passes `NullSink`:

```rust
            let ctx = Ctx {
                exec: &exec,
                fsys: &fsys,
                store: &store,
                paths: &paths,
                sink: &NullSink,
            };
```

Add the import beside the other `kuadrat_core` imports at the top:

```rust
use kuadrat_core::events::null::NullSink;
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --all`
Expected: PASS. The four new tests pass and **every pre-existing test still passes** — this task
changes no behaviour, only what is observable. A pre-existing failure means the harness changed a
scripted command; fix the harness, not the assertion.

- [ ] **Step 8: Run the full gate**

Run: `cargo fmt && cargo test --all && cargo clippy --all-targets -- -D warnings`
Expected: all pass; total **134**.

- [ ] **Step 9: Commit**

```bash
git add crates/core/src/deploy/ crates/cli/src/main.rs
git commit -m "feat(core): the deploy driver publishes events through the sink

Ctx gains the sink; begin/ok/fail funnel through one emit helper that
persists, takes the id back, and publishes. The order is load-bearing:
a reconnecting subscriber reads the store for its backlog, so an event
must be durable before anyone can see it.

The CLI passes NullSink — nothing is watching there, and the store is
the record."
```

---

### Task 4: Record the seam in ADR-0002 and reconcile the design document

**Files:**
- Modify: `docs/adr/0002-transport-agnostic-core.md`
- Modify: `docs/design/2026-08-11-phase-3-daemon-and-surfaces.md`
- Modify: `docs/known-gaps.md`

**Interfaces:**
- Consumes: everything from Tasks 1-3. No code changes.

This task exists because the ADR is what a future reviewer greps to decide whether a change is
allowed. An unrecorded seam reads as a violation of the rule, and the fix someone reaches for is to
delete it.

- [ ] **Step 1: Add the fourth clause to ADR-0002**

Open `docs/adr/0002-transport-agnostic-core.md` and find the numbered clauses stating the
no-direct-side-effects rule — clause 1 (`Executor`), clause 2 (`FileSystem`), clause 3 (the store
carve-out added in G1). Add a fourth **after** clause 3:

```markdown
4. `EventSink` is the third seam. Publishing an event to a subscriber is a side effect leaving
   `core`, so it goes through a trait rather than a channel type baked into a module — which is what
   keeps `tokio::sync::broadcast` a *daemon* dependency rather than a `core` one. `emit` is
   synchronous and returns nothing: a subscriber that has gone away must not be able to fail a
   deploy, and a sink with no way to await cannot block the deploy loop on I/O. A sink that needs
   async work hands off to a channel and does that work in its own task.
```

- [ ] **Step 2: Verify the ADR's grep rule still reads true**

Run: `grep -rn "std::fs\|Path::exists()" crates/core/src --include=*.rs | grep -v "#\[cfg(test)\]"`

The ADR notes that both appear legitimately in test blocks. This task does not change that. If the
output shows a **non-test** occurrence introduced by Tasks 1-3, that is a real seam violation — stop
and report it. Expected: no new occurrences beyond what was already there.

- [ ] **Step 3: Reconcile the design document with what was built**

In `docs/design/2026-08-11-phase-3-daemon-and-surfaces.md`, two passages now describe something
that was not built. Replace the `#[async_trait] async fn emit` code block in **The third seam**
with:

```rust
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &StoredEvent);
}
```

and replace the paragraph beginning "`emit` returns nothing." with:

```markdown
`emit` returns nothing and is synchronous. A subscriber that has gone away must not be able to fail
a deploy, so there is no error to propagate; and a sink with no way to await cannot block the deploy
loop on I/O. The signature carries both guarantees instead of a comment asking for them. A sink
needing async work — the H7 webhook — subscribes to the broadcast channel and does that work in its
own task.

Events reach a sink as `StoredEvent { id, event }` rather than as an `Event` carrying its own id.
An id only exists after the insert, so a type that carries one cannot be built before persisting:
persist-before-publish becomes a property of the types rather than a rule to remember.
```

In **What changes in `core`**, replace the `Event` gains `id: i64` bullet with:

```markdown
- `events`: `StoredEvent { id, event }`, the `EventSink` trait, `NullSink`, `FakeSink`.
  `store::append_event` returns the assigned id and `events_for` returns `StoredEvent`. **The column
  already exists** (`events.id INTEGER PRIMARY KEY AUTOINCREMENT`) — this is an API exposure change,
  not a schema change.
```

- [ ] **Step 4: Record what H1 deliberately did not do**

Add to `docs/known-gaps.md`, under a new `## From H1` heading:

```markdown
## From H1 — `reconcile` emits no events

`deploy::reconcile` rolls back crashed deploys and calls `finish_deploy`, but appends no events, so
a reconciled rollback is invisible to a subscriber. That is unchanged from phase 2 — reconcile never
emitted events — and H1 deliberately did not add them, because a watcher can only exist while the
daemon is running and reconcile runs before it binds. Revisit in H4 if the UI should show "this was
rolled back by a crash recovery" rather than only the terminal status.
```

- [ ] **Step 5: Verify the docs are consistent**

Re-read the three edited passages against `crates/core/src/events/mod.rs`. Every type and method
named in the design document must exist with that exact name and signature. A design doc that
describes a slightly different API is worse than one that describes none, because it gets trusted.

- [ ] **Step 6: Commit**

```bash
git add docs/
git commit -m "docs: record the EventSink seam in ADR-0002 and reconcile the design

The ADR is what a reviewer greps to decide whether a change is allowed;
an unrecorded seam reads as a violation and invites someone to delete
it. Also corrects the design document, which specified an async emit and
an id on Event — the call sites made both wrong, and a design doc that
describes a slightly different API is worse than none because it gets
trusted."
```

---

## H1 completion checklist

- [ ] `cargo test --all` passes with **134** tests (123 baseline + 11)
- [ ] `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean
- [ ] `EventSink` has exactly two implementations in `core`: `NullSink`, `FakeSink`
- [ ] `Ctx` has five fields; all 11 construction sites compile
- [ ] `append_event` returns the id; `events_for` returns `StoredEvent`
- [ ] A successful deploy emits 12 events, ordered, with ascending real ids
- [ ] Every emitted event is also in the store with the same id
- [ ] ADR-0002 has a fourth clause naming `EventSink`
- [ ] The design document describes the API that was actually built
- [ ] No new dependency in any `Cargo.toml`
- [ ] `grep -rn "tokio::sync::broadcast" crates/core/src` returns nothing — the broadcast sink is
      the daemon's, and its absence from `core` is the point of the seam

## Not in H1 (later groups)

| Group | What |
|---|---|
| H2 | `apps.repo_path`/`route` columns, `register_app`, the idempotent `ALTER TABLE` |
| H3 | The `logs` module — `tail`, `search` |
| H4 | `crates/daemon`, config, loopback guard, router, JSON API, the global semaphore |
| H5 | `BroadcastSink`, the SSE hub, backlog-then-live, dedupe, lag recovery, `Last-Event-ID` |
| H6 | The three htmx pages, embedded assets |
| H7 | Webhook sender, `kuadrat serve`, the systemd unit, `kuadrat deploy` as a client |
