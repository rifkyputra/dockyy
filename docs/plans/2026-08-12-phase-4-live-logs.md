# kuadrat Phase 4 · Live Log Tailing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** An operator on `/app/:name` presses **Follow** and watches the app's journal arrive live;
the same stream is available as JSON at `/api/apps/:name/logs/stream` for phase 4's agent.

**Architecture:** A fourth method on the `Executor` seam that yields a command's stdout as a stream
of lines, defaulting to `bail!` so a future SSH executor compiles until it opts in. `logs::follow`
runs the existing bounded `tail` as a pre-flight — because a stream loses the stderr correlation
that distinguishes "no entries" from "may not read the journal" — and then opens
`journalctl -f`. Two SSE endpoints render the same stream two ways, over a second, much simpler
engine than `events_sse`.

**Tech Stack:** Rust 2021, `tokio-stream` (new, `core`), axum 0.7, `journalctl`.

**Design:** [`docs/design/2026-08-11-phase-4-live-logs.md`](../design/2026-08-11-phase-4-live-logs.md)

## Global Constraints

- **`core` never opens a socket and never takes a `host` parameter** (ADR-0002). This group adds a
  *method* to the existing seam, not a second way to reach the host.
- **One new dependency, `core` only:** `tokio-stream = { version = "0.1", features = ["io-util"] }`.
  The `io-util` feature is what provides `wrappers::LinesStream`; without it the wrapper does not
  exist and you would be hand-rolling `poll_next`, which is the thing this dependency exists to
  avoid. Nothing else, and nothing new in `crates/daemon` or `crates/cli`.
- **Bounded by construction.** The follow's backlog is clamped against the existing
  `logs::MAX_LINES`, as `tail` and `search` already are. The stream itself is bounded by client
  disconnect *and* a duration ceiling — both, not either.
- **`maud::PreEscaped` appears nowhere.** A log line is the least trusted string in the system.
- **No adblock-bait substrings** in any DOM id or class: no `ad`, `ads`, `banner`, `popup`, `promo`,
  `sponsor`, `social`, `consent`, `cookie`, `newsletter`. Adblocker cosmetic filters hide matching
  elements in the *user's* browser — invisible to curl and to tests.
- **`make check` must pass**: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
- **Prefix cargo commands with `PATH=$HOME/.cargo/bin:$PATH`.**
- **Baselines, measured at `22ffc46`:** `kuadrat` (cli) **30**, `kuadrat_core` **195**,
  `kuadrat_daemon` **80**.
- Commit after every task with a Conventional Commit subject.

## Fixed quantities, from the design

- **The duration ceiling is 30 minutes.** `EventSource` reconnects on its own, so a viewer still
  watching sees a gap of a second rather than an ended stream.
- **The follow's backlog is 100 lines** — the same figure the static tail and the JSON logs endpoint
  already use.

## The one thing that is easy to get wrong

`logs::tail` distinguishes "this app has logged nothing" from "this process may not read the system
journal" by inspecting **stderr**: journald prints the privilege hint there while still printing
`-- No entries --` to stdout and exiting 0.

A stream carrying stdout loses that — not the data, the *correlation*. So `follow` runs `tail` once
first. That detection is already written and already tested; re-implementing a weaker version
against a data shape that cannot support it is the mistake this plan exists to prevent.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/Cargo.toml` | *Modify.* `tokio-stream` |
| `crates/core/src/exec/mod.rs` | *Modify.* `run_streaming` on the trait, with a bailing default |
| `crates/core/src/exec/local.rs` | *Create impl.* Spawn, pipe stdout, own the `Child` |
| `crates/core/src/exec/fake.rs` | *Create impl.* Scripted lines |
| `crates/core/src/logs/mod.rs` | *Modify.* `follow`, with its pre-flight |
| `crates/daemon/src/stream.rs` | *Modify.* `lines_sse` beside `events_sse` |
| `crates/daemon/src/api.rs` | *Modify.* `GET /api/apps/:name/logs/stream` |
| `crates/daemon/src/pages.rs` | *Modify.* `GET /app/:name/logs/stream`, the Follow control |
| `docs/known-gaps.md`, `README.md` | *Modify.* Record what landed |

---

### Task 1: The streaming seam

**Files:**
- Modify: `crates/core/Cargo.toml`, `crates/core/src/exec/mod.rs`,
  `crates/core/src/exec/local.rs`, `crates/core/src/exec/fake.rs`

**Interfaces:**
- Produces:
  - `Executor::run_streaming(&self, program: &str, args: &[String]) -> Result<Box<dyn Stream<Item = Result<String>> + Send + Unpin>>`
    — a default impl that `bail!`s
  - `LocalExecutor` and `FakeExecutor` implementations
  - `FakeExecutor::expect_stream(program: &str, lines: Vec<String>)`

- [x] **Step 1: Add the dependency**

`crates/core/Cargo.toml`:

```toml
tokio-stream = { version = "0.1", features = ["io-util"] }
```

Add it to `[workspace.dependencies]` in the root `Cargo.toml` too, matching how the other shared
crates are declared, and reference it as `tokio-stream.workspace = true`.

- [x] **Step 2: Write the failing tests**

In `crates/core/src/exec/mod.rs`'s test module:

```rust
    /// A new executor — the fleet driver's SSH one, later — must compile
    /// before it supports streaming. The default impl is what allows that,
    /// and it must fail loudly rather than silently yielding nothing.
    #[tokio::test]
    async fn an_executor_that_has_not_opted_in_bails_on_run_streaming() {
        struct Minimal;
        #[async_trait::async_trait]
        impl Executor for Minimal {
            async fn run(&self, _p: &str, _a: &[String]) -> anyhow::Result<CommandOutput> {
                unreachable!()
            }
        }
        let err = Minimal.run_streaming("journalctl", &[]).await.unwrap_err();
        assert!(err.to_string().contains("streaming"), "was: {err}");
    }

    #[tokio::test]
    async fn the_local_executor_yields_stdout_a_line_at_a_time() {
        use tokio_stream::StreamExt;
        let exec = LocalExecutor;
        let mut stream = exec
            .run_streaming("sh", &["-c".into(), "printf 'one\\ntwo\\nthree\\n'".into()])
            .await
            .expect("stream");

        let mut got = Vec::new();
        while let Some(line) = stream.next().await {
            got.push(line.expect("line"));
        }
        assert_eq!(got, vec!["one", "two", "three"]);
    }

    /// Dropping the stream must kill the child. Without that, "the client went
    /// away" is an intention rather than a bound, and every abandoned viewer
    /// leaves a `journalctl -f` running.
    #[tokio::test]
    async fn dropping_the_stream_kills_the_child() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pidfile = dir.path().join("pid");
        let script = format!("echo $$ > {}; sleep 30", pidfile.display());

        let exec = LocalExecutor;
        let stream = exec
            .run_streaming("sh", &["-c".into(), script])
            .await
            .expect("stream");

        // Wait for the child to record its pid rather than sleeping a fixed
        // amount: a sleep is slow when it passes and flaky when it does not.
        let pid = loop {
            if let Ok(text) = std::fs::read_to_string(&pidfile) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    break pid;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        };

        drop(stream);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::path::Path::new(&format!("/proc/{pid}")).exists() {
            assert!(std::time::Instant::now() < deadline, "child {pid} outlived its stream");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn the_fake_executor_yields_its_scripted_lines() {
        use tokio_stream::StreamExt;
        let exec = FakeExecutor::new();
        exec.expect_stream("journalctl", vec!["a".into(), "b".into()]);

        let mut stream = exec.run_streaming("journalctl", &[]).await.expect("stream");
        let mut got = Vec::new();
        while let Some(line) = stream.next().await {
            got.push(line.expect("line"));
        }
        assert_eq!(got, vec!["a", "b"]);
    }
```

`tempfile` is already a dev-dependency of `crates/core`.

- [x] **Step 3: Run to verify they fail**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test -p kuadrat-core exec:: 2>&1 | tail -20
```

Expected: compile failure — `no method named run_streaming`.

- [x] **Step 4: Add the trait method**

In `crates/core/src/exec/mod.rs`, beside `run_with_stdin`:

```rust
    /// Run a command and yield its stdout a line at a time, for as long as it
    /// runs.
    ///
    /// Returns a stream rather than taking a channel because a channel-based
    /// signature does not return until the stream ends, so a caller cannot
    /// both drive it and read it in one task — it would have to spawn, and
    /// `spawn` needs `'static` while `core` holds `&dyn Executor` everywhere.
    /// A seam that dictates its caller's task structure has stopped being an
    /// abstraction. The `Result` per item puts a mid-stream failure inline,
    /// where it happened, instead of on a separate path from the lines it
    /// interrupted.
    ///
    /// Default impl bails, like `run_with_stdin`, so a new executor compiles
    /// until it opts in.
    async fn run_streaming(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<Box<dyn Stream<Item = Result<String>> + Send + Unpin>> {
        let _ = (program, args);
        anyhow::bail!("streaming is not supported by this executor")
    }
```

with `use tokio_stream::Stream;` at the top.

- [x] **Step 5: Implement it for `LocalExecutor`**

```rust
/// A child's stdout, line by line, with the child kept alive alongside it.
///
/// The `Child` is held here rather than detached so that dropping the stream
/// drops the child — and `kill_on_drop(true)` then kills it. That is what makes
/// "the consumer went away" an enforced bound rather than an intention.
struct ChildLines {
    _child: Child,
    lines: LinesStream<BufReader<ChildStdout>>,
}

impl Stream for ChildLines {
    type Item = Result<String>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.lines)
            .poll_next(cx)
            .map(|item| item.map(|r| r.map_err(anyhow::Error::from)))
    }
}
```

Both fields are `Unpin`, so `Pin::new(&mut self.lines)` needs no unsafe pinning — that is precisely
what `tokio-stream` buys here.

`run_streaming` spawns with `.stdout(Stdio::piped())` and `.kill_on_drop(true)`, takes
`child.stdout.take()`, wraps it in `BufReader`, calls `.lines()`, wraps that in
`LinesStream::new`, and returns `Box::new(ChildLines { .. })`.

- [x] **Step 6: Implement it for `FakeExecutor`**

Record the call the way the other methods do, then return
`Box::new(tokio_stream::iter(lines.into_iter().map(Ok)))`. `expect_stream` stores the scripted lines
per program beside the existing `expect`/`expect_call` maps; an unscripted program returns an error
naming it, matching how the fake already behaves.

- [x] **Step 7: Run the suite**

Expected: core **199** (195 + 4), daemon 80, cli 30. `make check` clean.

- [x] **Step 8: Commit**

```bash
git add crates/core Cargo.toml Cargo.lock
git commit -m "feat(core): a streaming seam on the Executor"
```

---

### Task 2: `logs::follow`

**Files:**
- Modify: `crates/core/src/logs/mod.rs`

**Interfaces:**
- Consumes: `Executor::run_streaming`, the existing `tail`, `MAX_LINES`, `unit_name`
- Produces: `pub async fn follow(exec: &dyn Executor, name: &str, lines: usize) -> Result<Box<dyn Stream<Item = Result<String>> + Send + Unpin>>`

- [x] **Step 1: Write the failing tests**

```rust
    #[tokio::test]
    async fn follow_asks_journalctl_to_follow_the_prefixed_unit() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "-- No entries --\n", ""));   // the pre-flight
        exec.expect_stream("journalctl", vec!["line one".into()]);

        follow(&exec, "web", 100).await.expect("follow");

        let (_, args) = &exec.calls()[1];
        assert!(args.iter().any(|a| a == "-u"), "{args:?}");
        assert!(args.iter().any(|a| a == "kuadrat-web"), "{args:?}");
        assert!(args.iter().any(|a| a == "-f"), "{args:?}");
    }

    /// The pre-flight exists for exactly this: journald reports an unreadable
    /// journal on *stderr* while exiting 0, so a stream of stdout alone cannot
    /// tell it from an app that has logged nothing. `tail` already detects it.
    #[tokio::test]
    async fn an_unreadable_journal_fails_before_any_stream_opens() {
        let exec = FakeExecutor::new();
        exec.expect(
            "journalctl",
            out(0, "-- No entries --\n", "Hint: You are currently not seeing messages from other users and the system.\n"),
        );

        let err = follow(&exec, "web", 100).await.unwrap_err();
        assert!(err.to_string().contains("journal"), "was: {err}");
        assert_eq!(exec.calls().len(), 1, "the stream must not have been opened");
    }

    #[tokio::test]
    async fn follows_backlog_is_clamped_like_every_other_read() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "", ""));
        exec.expect_stream("journalctl", vec![]);

        follow(&exec, "web", MAX_LINES + 500).await.expect("follow");

        let (_, args) = &exec.calls()[1];
        let n = args.iter().position(|a| a == "-n").map(|i| &args[i + 1]).expect("-n");
        assert_eq!(n, &MAX_LINES.to_string());
    }
```

- [x] **Step 2: Run to verify they fail**

- [x] **Step 3: Implement**

```rust
/// Follow a workload's journal: the last `lines` entries, then everything that
/// arrives after.
///
/// Runs the bounded [`tail`] once first, and fails if it does. That is not
/// redundancy: journald reports "you may not read this journal" on stderr while
/// still exiting 0 and printing `-- No entries --` to stdout, so a stream
/// carrying stdout alone cannot tell that apart from a quiet app. `tail`
/// already makes that distinction and is already tested for it; the pre-flight
/// borrows a correct detection rather than writing a weaker one against a data
/// shape that cannot support it.
pub async fn follow(
    exec: &dyn Executor,
    name: &str,
    lines: usize,
) -> Result<Box<dyn Stream<Item = Result<String>> + Send + Unpin>> {
```

The body: `tail(exec, name, lines).await?;` then `exec.run_streaming("journalctl", &args).await`
with `-u <unit> -f -n <clamped>` and the same no-`-q` reasoning the module already documents.

- [x] **Step 4: Run the suite**

Expected: core **202** (199 + 3), daemon 80, cli 30.

- [x] **Step 5: Commit**

```bash
git add crates/core/src/logs/mod.rs
git commit -m "feat(core): follow a workload's journal"
```

---

### Task 3: `lines_sse` and the JSON endpoint

**Files:**
- Modify: `crates/daemon/src/stream.rs`, `crates/daemon/src/api.rs`

**Interfaces:**
- Consumes: `logs::follow`
- Produces:
  - `pub fn lines_sse<F>(stream: Box<dyn Stream<Item = Result<String>> + Send + Unpin>, render: F, deadline: Duration) -> Response`
    where `F: Fn(&str) -> String + Send + 'static`
  - route `GET /api/apps/:name/logs/stream`

`lines_sse` is a **second, simpler engine** than `events_sse`, not a reuse of it: log lines have no
store ids, so there is nothing to deduplicate, nothing to resume from, and nothing to re-read after
a lag. Reusing `events_sse` would mean inventing ids to drive machinery protecting a property log
lines do not have.

It shares `events_sse`'s *shape* — a renderer parameter, and the payload sanitised in the engine so
no renderer can produce a `data` field that `sse::Event::data` rejects.

- [x] **Step 1: Write the failing tests**

```rust
    #[tokio::test]
    async fn the_log_stream_sends_one_event_per_line() {
        let (app, store, _hub, _d) = harness_with_journal(vec!["one".into(), "two".into()]);
        register(&store, "web");

        let res = app
            .oneshot(get("/api/apps/web/logs/stream"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::OK);

        let data = sse_raw_data(res).await;
        assert_eq!(data.len(), 2);
        assert!(data[0].contains("one"), "{}", data[0]);
    }

    #[tokio::test]
    async fn an_unregistered_app_is_a_404_before_any_stream() {
        let (app, _store, _hub, _d) = harness_parts();
        let res = app.oneshot(get("/api/apps/nope/logs/stream")).await.expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    /// A line carrying a carriage return must not panic the stream:
    /// `sse::Event::data` asserts on `\r`, and journald carries whatever an
    /// application wrote. This is the same defect H6 shipped and fixed once;
    /// the sanitising belongs in the engine so a second renderer cannot
    /// reintroduce it.
    #[tokio::test]
    async fn a_line_containing_a_carriage_return_does_not_panic() {
        let (app, store, _hub, _d) = harness_with_journal(vec!["a\rb".into()]);
        register(&store, "web");

        let res = app.oneshot(get("/api/apps/web/logs/stream")).await.expect("send");
        let data = sse_raw_data(res).await;
        assert_eq!(data.len(), 1);
        assert!(!data[0].contains('\r'));
    }
```

Plus one for the ceiling, which is otherwise the only fixed quantity in this group with no test:

```rust
    /// The ceiling exists for the half-dead connection the server never
    /// notices dropping. Call the engine directly with a tiny deadline rather
    /// than waiting thirty minutes; the paused clock keeps it free.
    #[tokio::test(start_paused = true)]
    async fn a_stream_ends_when_its_deadline_elapses() {
        // A source that never ends: without the deadline this would hang.
        let never = Box::new(tokio_stream::pending::<Result<String>>());
        let res = lines_sse(never, |l| l.to_string(), Duration::from_secs(1));

        let body = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("the deadline must end the body");
        assert!(body.is_empty());
    }
```

`harness_with_journal(lines)` is a variant of `harness_parts` whose `FakeExecutor` scripts both the
pre-flight `tail` (exit 0, empty stderr) and the stream. `register(&store, name)` is a one-line
helper writing an `AppConfig`; if an equivalent already exists in the test module, use it rather
than adding a second.

- [x] **Step 2: Run to verify they fail**

- [x] **Step 3: Implement**

`lines_sse` builds an `async_stream::stream!` that yields each line as an `sse::Event`, ends when
the source ends, and is wrapped in `tokio::time::timeout(deadline, ..)` semantics — when the ceiling
elapses the stream simply ends, which closes the connection normally.

Sanitise in the engine, as `to_sse_event` already does for events:

```rust
    // `sse::Event::data` splits on `\n` but *asserts* on `\r`, and a journal
    // line carries whatever the application wrote. Sanitising here rather than
    // in a renderer means a future third caller inherits it.
    .data(payload.replace(['\r', '\n'], " "))
```

The JSON renderer emits `serde_json::to_string(&serde_json::json!({ "line": line }))`.

The handler 404s for an unregistered app before calling `follow`, as the existing logs endpoint does.

- [x] **Step 4: Run the suite**

Expected: daemon **84** (80 + 4), core 202, cli 30.

- [x] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): stream a workload's journal as JSON"
```

---

### Task 4: The page's Follow control

**Files:**
- Modify: `crates/daemon/src/pages.rs`

**Interfaces:**
- Consumes: `lines_sse`, `logs::follow`
- Produces: `GET /app/:name/logs/stream` (HTML fragments), and the Follow control on `/app/:name`

- [x] **Step 1: Write the failing tests**

```rust
    /// Follow is a control the operator presses, not behaviour on load — the
    /// same judgement H6 made about the app list not refreshing itself.
    /// Content that moves under a reader is worse than content that is stale,
    /// unless the reader asked for it.
    #[tokio::test]
    async fn the_app_page_offers_follow_without_attaching_a_stream() {
        let (app, store, _hub, _d) = harness_parts();
        register(&store, "web");

        let body = body_text(app.oneshot(get("/app/web")).await.expect("send")).await;
        assert!(body.to_lowercase().contains("follow"), "no control: {body}");
        assert!(!body.contains("sse-connect"), "the page must not attach on load");
    }

    #[tokio::test]
    async fn the_log_fragment_stream_sends_rows_not_json() {
        let (app, store, _hub, _d) = harness_with_journal(vec!["hello".into()]);
        register(&store, "web");

        let res = app.oneshot(get("/app/web/logs/stream")).await.expect("send");
        let data = sse_raw_data(res).await;
        assert!(data[0].starts_with("<li"), "fragment: {}", data[0]);
        assert!(data[0].contains("hello"));
    }

    /// The least trusted string in the system, arriving live.
    #[tokio::test]
    async fn a_streamed_log_line_containing_markup_is_escaped() {
        let (app, store, _hub, _d) = harness_with_journal(vec!["<script>alert(1)</script>".into()]);
        register(&store, "web");

        let res = app.oneshot(get("/app/web/logs/stream")).await.expect("send");
        let data = sse_raw_data(res).await;
        assert!(!data[0].contains("<script>alert(1)</script>"), "raw markup: {}", data[0]);
        assert!(data[0].contains("&lt;script&gt;"));
    }
```

- [x] **Step 2: Run to verify they fail**

- [x] **Step 3: Implement**

A `log_line(line: &str) -> Markup` rendering one `<li class="log-line">`, used by the stream's
renderer.

**The Follow control is a link to the same page, not a new route.** `GET /app/:name` accepts an
optional `follow` query parameter: without it the page renders the static tail and a
`<a class="log-follow" href="/app/web?follow=1">Follow</a>`; with it, the page renders the
sse-connected `<ul>` carrying `hx-ext="sse"`, `sse-connect="/app/web/logs/stream"`,
`sse-swap="message"` and `hx-swap="beforeend"`.

One route, no third endpoint, and no JavaScript beyond the htmx the layout already loads. The page
as first rendered carries no `sse-connect` at all, which is what the first test pins — and the
operator's choice to follow is in the URL, so it survives a reload and can be linked to.

Class names: `log-line`, `log-follow`. Check them against the adblock-bait list before committing —
none of them contain a banned substring, and any name you add instead must not either.

- [x] **Step 4: Run the suite**

Expected: daemon **87** (84 + 3), core 202, cli 30.

- [x] **Step 5: Commit**

```bash
git add crates/daemon/src/pages.rs
git commit -m "feat(daemon): follow an app's log from its page"
```

---

### Task 5: Record what landed

**Files:**
- Modify: `docs/known-gaps.md`, `README.md`

- [x] **Step 1: Close H3's deferral**

`docs/known-gaps.md` records that live tailing was deferred to phase 4. Replace that with what
shipped: the seam, `follow`, the two endpoints, the 30-minute ceiling, and that the second consumer
— the MCP surface — is still to come.

- [x] **Step 2: Record the new gap**

```markdown
## From phase 4 — a followed stream holds a `journalctl` for up to 30 minutes

Each viewer following a log holds one `journalctl -f` process. Dropping the stream kills it, and a
30-minute ceiling bounds the connection the server never notices dropping — but a host with several
operators watching several apps holds one process each for as long as they watch.

That is the intended cost of live tailing and not a defect. It is recorded because the premise of
this project is a low-memory host, and "how many followers is too many" has never been measured.
```

- [x] **Step 3: Update the README**

Add live log following to what the web UI does, and the JSON endpoint to what an API client can
consume. Keep it to the README's existing voice — a sentence, not a section.

- [x] **Step 4: Commit**

```bash
git add docs README.md
git commit -m "docs: record live log tailing and what it costs"
```

---

## Completion checklist

> Closed 2026-08-18, verified on sumo. Daemon landed at **90** tests, not the 87 planned: the
> whole-branch review polish (`240011b`) and the follow-mode fix (`90db30f`) added three beyond
> this plan. `PreEscaped` appears only in two doc comments that state the rule; zero code usages.
> Daemon's `tokio-stream` is `[dev-dependencies]` only (the deadline test's `pending`), so the
> production tree still has exactly one new dependency, in `core`.

- [x] `cargo test --workspace` passes: core 202, daemon 87, cli 30 — measured: core 202, daemon 90, cli 30, 0 failed
- [x] `make check` clean
- [x] `maud::PreEscaped` appears nowhere in the repository
- [x] No DOM id or class contains an adblock-bait substring
- [x] A streamed log line containing markup renders as text, proven by a test
- [x] Dropping a stream kills its child, proven by a test
- [x] An unreadable journal fails before any stream opens, proven by a test
- [x] One new dependency, `core` only

## Not in this group

- **The MCP surface** — the second consumer, which uses `/api/apps/:name/logs/stream`.
- **The fleet driver** — a remote executor implements `run_streaming` over SSH, which is why the
  default impl bails rather than being required.
- **Authentication and CSRF** — still recorded with their shared trigger.
- **The unresolved Healthcheck hang** from H7's acceptance run — recorded separately; it does not
  touch this code.
