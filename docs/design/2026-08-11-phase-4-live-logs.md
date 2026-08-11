# Phase 4 · Live Log Tailing

The first group of phase 4, and the one the other two rest on: a streaming seam in `core`, and a
live log tail built on it.

Phase 3's H3 deferred this deliberately — "Live tailing is deferred to phase 4, which needs a
streaming seam this group deliberately does not build" — and the parent design named the reason it
was worth waiting for: two consumers. The operator's page is the first. Phase 4's MCP surface is the
second, and it will want live logs to diagnose a failure.

Prior art in this repository: [`2026-08-11-phase-3-daemon-and-surfaces.md`](./2026-08-11-phase-3-daemon-and-surfaces.md),
[`2026-08-11-phase-3-h6-pages.md`](./2026-08-11-phase-3-h6-pages.md) (the two-renderers-one-engine
pattern), and [`../adr/0002-transport-agnostic-core.md`](../adr/0002-transport-agnostic-core.md)
(why every host interaction goes through a seam).

## Goal

An operator on `/app/:name` presses **Follow** and watches the app's journal arrive live. The same
stream is available as JSON at `/api/apps/:name/logs/stream`, so phase 4's agent consumes it without
scraping a page.

## The two decisions this group makes

| Decision | Choice | Why |
|---|---|---|
| Streaming seam shape | **`Box<dyn Stream<Item = Result<String>>>`** with `tokio-stream` in `core` | A channel-based seam forces every caller to `spawn`, which needs `'static` and so forces `core` off `&dyn Executor`. A seam that dictates its caller's task structure has stopped being an abstraction |
| What bounds a tail | **Client disconnect, plus a duration ceiling** | Drop kills the child, which makes disconnect a real bound rather than a promise. The ceiling catches the half-dead connection the server never notices — the leak that is invisible until the host runs out of something |

### Why not a channel

`async fn run_lines(&self, .., tx: Sender<String>)` does not return until the stream ends, so a
caller cannot both drive it and read the channel in one task. It must spawn one side, and `spawn`
requires `'static` — but `core` holds `&dyn Executor` everywhere, deliberately, because that is what
lets `FakeExecutor` be injected and a future SSH executor be substituted.

There is a second cost. A channel-based seam reports failure through its return value, so an error
that interrupts the stream mid-flight arrives on a different path from the lines it interrupted. A
`Stream<Item = Result<String>>` puts the failure inline, where it happened.

The price is one small first-party crate. The alternative to `tokio-stream` is not "no dependency" —
it is hand-written `poll_next` and pinning over `tokio::io::Lines`, in the seam every host
interaction flows through. That is the wrong thing to hand-roll.

This is a different judgement from H7's refusal of `reqwest`, and the difference is the point: there
the alternative was `curl`, already on every host that runs podman, against ~100 crates. Here the
alternative is writing by hand what someone else has already written correctly.

## The seam

```rust
/// Run a command and yield its stdout a line at a time, for as long as it runs.
///
/// Default impl bails, like `run_with_stdin`, so a new executor compiles until
/// it opts in.
async fn run_streaming(
    &self,
    program: &str,
    args: &[String],
) -> Result<Box<dyn Stream<Item = Result<String>> + Send + Unpin>>;
```

`LocalExecutor` spawns with `kill_on_drop(true)` and returns a stream that **owns the `Child`**.
Dropping the stream kills the process. That is what makes "the client went away" an enforced bound
rather than an intention — nothing has to remember to clean up.

`FakeExecutor` returns `tokio_stream::iter` over scripted lines, and records the call like its
siblings do.

## The wrinkle: stderr does not survive streaming

`logs::tail` distinguishes "this app has logged nothing" from "this process may not read the system
journal" by inspecting **stderr** — journald prints the privilege hint there while still printing
`-- No entries --` to stdout and exiting 0. H3 wrote a test for exactly that, and its reasoning is
recorded: in a root-privileged tool the two must not look alike.

A stream carrying stdout loses that. Not the data — the *correlation*. `CommandOutput` delivers
stdout, stderr and status together at the end; a stream delivers one of them, gradually, and nothing
says which stderr line explains which stdout line.

**So `follow` runs the existing bounded `tail` once before opening the stream.** It already detects
the privilege case and is already tested. The cost is one extra `journalctl` invocation per follow;
what it buys is error detection that is known-correct, instead of a weaker second implementation
written against a data shape that cannot support it.

## `core::logs::follow`

```rust
pub async fn follow(
    exec: &dyn Executor,
    name: &str,
    lines: usize,
) -> Result<Box<dyn Stream<Item = Result<String>> + Send + Unpin>>;
```

Runs `journalctl -u kuadrat-<slug> -f -n <lines>` — backlog then live, the same shape the event
stream has, for the same reason: a viewer who arrives mid-incident should see what already happened.

`lines` is clamped against the existing `MAX_LINES`, as `tail` and `search` are. The follow itself
is unbounded by nature; its backlog is not, and there is no reason to make an exception.

## Surfaces

| Route | Serves |
|---|---|
| `GET /api/apps/:name/logs/stream` | JSON, one line per event — the CLI and phase 4's agent |
| `GET /app/:name/logs/stream` | HTML fragments — the page, via htmx |

Two renderers over one engine, as H6 established. But a **second, much simpler engine** than
`events_sse`: log lines have no store ids, so there is nothing to deduplicate, nothing to resume
from, and nothing to re-read after a lag. `lines_sse(stream, render, deadline)` sits beside
`events_sse` in `stream.rs` and shares its shape, not its body.

Reusing `events_sse` would mean inventing ids for lines that have none, to drive machinery that
protects a property log lines do not have.

## The page

`/app/:name` keeps its static 100-line tail. **Follow is a control the operator presses**, not
behaviour on load.

That is the same judgement H6 made about the app list not refreshing itself: content that moves
under a reader is worse than content that is a few seconds stale — unless the reader asked for it.
An operator opening the app page is usually reading the route, the image, or the deploy history;
having the log start scrolling underneath is a cost paid on every visit for a benefit wanted on
some.

## Fixed quantities

Named here rather than left to the implementer, so two people reading this build the same thing:

- **The duration ceiling is 30 minutes.** Long enough that an operator working an incident never
  meets it; short enough that a tab forgotten overnight holds a `journalctl` process for half an
  hour rather than until the host reboots. `EventSource` reconnects on its own, so a viewer who is
  still watching sees a gap of a second, not an ended stream.
- **The follow's backlog is 100 lines** — the same figure the static tail and the JSON logs endpoint
  already use, so all three mean the same thing by "the recent log".

## Error handling

| Condition | Behaviour |
|---|---|
| journal unreadable | caught by the pre-flight `tail`; the page renders the existing `log-error` note, the API returns an error before the stream opens |
| stream dies mid-follow | the connection closes; the browser reconnects and the pre-flight runs again |
| duration ceiling reached | the stream closes normally; a browser that is still watching reconnects, which is the intended cost |
| client disconnects | the stream drops, `kill_on_drop` kills `journalctl` |

## Testing

`FakeExecutor` scripts the lines, so the seam, `follow`, and both handlers are testable with no
journal and no host. Specifically worth pinning: that dropping the stream is what stops the process
(a test that the fake records the drop), that the pre-flight failure surfaces before any streaming
begins, and that the deadline closes an otherwise-live stream.

The escaping rule from H6 still binds: a log line is the least trusted string in the system, and
`maud::PreEscaped` appears nowhere.

## Not in this group

- **The MCP surface** — the second consumer. It uses `/api/apps/:name/logs/stream` when it lands.
- **The fleet driver** — a remote executor would implement `run_streaming` over SSH, which is why
  the default impl bails rather than being required.
- **Authentication and CSRF** — still recorded in `known-gaps.md` with their shared trigger.
