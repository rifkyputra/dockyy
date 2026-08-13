# Phase 5 · The MCP Surface

The fourth consumer named in [`ADR-0002`](../adr/0002-transport-agnostic-core.md) — "a daemon, a CLI,
a web UI, and an MCP surface for agents" — and the last one still unbuilt. The README's *Why* names
"no agent-operable interface" as a gap kuadrat exists to close, and its status line says plainly
that there is no MCP surface yet.

Phase 4 built the second consumer's plumbing before the consumer: `logs::follow` and
`/api/apps/:name/logs/stream` exist because
[`2026-08-11-phase-4-live-logs.md`](./2026-08-11-phase-4-live-logs.md) named the agent as the
stream's other reader. This phase is that reader.

Prior art in this repository: [`2026-08-11-phase-3-daemon-and-surfaces.md`](./2026-08-11-phase-3-daemon-and-surfaces.md)
(the daemon as a surface over `core`), [`2026-08-11-phase-3-h7-serve.md`](./2026-08-11-phase-3-h7-serve.md)
(the daemon-first-with-fallback pattern `kuadrat deploy` already uses), and
[`../adr/0002-transport-agnostic-core.md`](../adr/0002-transport-agnostic-core.md).

## Goal

An agent — Claude Code, or anything else speaking MCP — can ask a kuadrat host what is deployed,
deploy an app, watch the deploy's stages, and read a workload's journal, without a human relaying
CLI output into a chat window.

## The four decisions this group makes

| Decision | Choice | Why |
|---|---|---|
| Transport | **stdio only**, as `kuadrat mcp` | An HTTP MCP server is a second network surface on a daemon that has no authentication, which trips the trigger `known-gaps.md` records for auth and CSRF. stdio has no listener: the client spawns the process, and the OS is the boundary |
| What it talks to | **The daemon over loopback, and it refuses to start without one** | Deploys must serialise behind the store's per-app lock. The CLI can fall back in-process because a human sees the fallback message; an agent cannot, and a silent second code path is worse than a refusal |
| The tool surface | **Read-heavy, plus `deploy` and `reconcile`. No secrets, no remove** | A tool the agent cannot invoke is a tool that cannot be misinvoked. Secrets are stdin-only by construction; MCP has no stdin |
| The protocol implementation | **Hand-rolled JSON-RPC over stdio in a new `crates/mcp`** | The subset needed is `initialize`, `tools/list`, `tools/call` and one notification. That is smaller than the code an SDK's own error handling would add, and this repository refused `reqwest` for less |

### Why not an HTTP MCP server

MCP defines a streamable-HTTP transport, and it is tempting: the daemon is already an HTTP server,
and one more route looks cheaper than one more crate.

It is not cheaper, because of what `known-gaps.md:237-251` records. `POST /apps` and the browser
branch of the deploy route have no CSRF defence, deliberately, and the gap says why: the phase binds
loopback and ships no authentication at all, so a token "would be the only control on a surface that
has no others, and it would suggest a boundary that is not there." It then names the trigger — those
must be fixed "in the same change that gives the daemon authentication or reachability beyond
loopback — whichever comes first."

An HTTP MCP endpoint is a new reachable, state-changing surface. It trips that trigger, and phase 5
becomes an auth phase wearing an MCP hat.

stdio does not trip it. The client spawns `kuadrat mcp` as a child process and speaks to it over the
pipe it owns. There is no port, no origin, and nothing for a browser to submit a form to. The
security boundary is the one the operating system already draws around process spawning — the same
boundary that governs `kuadrat deploy` today.

### Why it requires a daemon rather than falling back

`kuadrat deploy` tries the daemon and falls back to an in-process deploy when none answers.
[`daemon_client.rs`](../../crates/cli/src/daemon_client.rs) documents why that is safe: "the daemon
and a local run share one SQLite file and its per-app lock."

So the fallback is *correct*. It is still wrong here, for a different reason: an operator reading
`kuadrat deploy`'s output sees which path ran. An agent gets a tool result, and a tool result that
means two different things depending on invisible host state is a tool an agent will reason about
incorrectly. The failure is not corruption; it is an agent confidently reporting a deploy the
operator cannot find in the daemon's timeline.

`kuadrat mcp` therefore probes the daemon at startup and exits with a message naming
`kuadrat serve` if nothing answers. One path, always.

## The tool surface

| Tool | Maps to | Notes |
|---|---|---|
| `list_apps` | `GET /api/apps` | Name, repo path, route, host status |
| `get_app` | `GET /api/apps/:name` | 404 becomes a tool error, not an empty result |
| `deploy` | `POST /api/apps/:name/deploy` | Returns the `deploy_id` immediately; does not block |
| `get_deploy` | `GET /api/deploys/:id` | Stage, status, detail, and the event list |
| `tail_logs` | `GET /api/apps/:name/logs` | Bounded read; `n` clamped by `core` as it already is |
| `reconcile` | The CLI's `reconcile` path | Rolls back anything left in progress after a crash |

Deliberately absent:

- **`remove`** — the one irreversible operation. An agent that misreads a name deletes the wrong
  workload, and there is no undo. A human runs `kuadrat remove`.
- **`secret set` / `secret rm`** — `kuadrat secret` reads values from stdin, never argv, so they stay
  out of process listings and shell history. An MCP tool call is a JSON object in a transcript. The
  property that makes the CLI's design safe is the one MCP cannot provide.
- **`follow_logs`** — see below.

### The one that does not fit: live logs

Phase 4's design says the MCP surface "uses `/api/apps/:name/logs/stream` when it lands," and the
JSON endpoint was built for it. Taking that literally does not work, and the mismatch is worth
stating rather than discovering during implementation.

An MCP tool call is request/response: the agent calls, the server returns one result, the turn
continues. There is no shape in `tools/call` for "and then keep sending things for thirty minutes."
MCP has progress notifications, but they carry progress, not payload, and an agent cannot reason
over a stream it has to accumulate across notifications anyway.

What an agent diagnosing a failure actually wants is a *bounded snapshot it can read in one turn* —
which is `tail_logs`, and which already exists. So phase 5 ships `tail_logs` and not `follow_logs`,
and the streaming endpoint keeps serving the page.

That is not wasted work from phase 4. The streaming seam is what the fleet driver's SSH executor
implements, and `logs::follow`'s pre-flight is what makes an unreadable journal legible on both
paths. The endpoint's second consumer is simply later than phase 4 predicted.

## Protocol

MCP is JSON-RPC 2.0 over a line-delimited stdio pipe. The subset this surface needs:

- `initialize` — the client sends its protocol version and capabilities; the server answers with its
  own and with server info. The version is a dated string that both sides negotiate.
- `notifications/initialized` — the client's acknowledgement. No response.
- `tools/list` — returns the table above, each tool carrying a name, a description, and a JSON Schema
  for its input.
- `tools/call` — name plus arguments; returns content blocks, or an error.

**Read the current protocol version out of the specification when implementing this, rather than
from this document.** MCP revises its version string on a date cadence, and a version pinned in a
design doc written months earlier is exactly the kind of stale constant this repository treats as a
bug. The negotiation is the contract; the string is not.

Anything else the client sends gets a JSON-RPC `method not found`. Resources and prompts are not
implemented — the surface is tools.

## Where it lives

A new `crates/mcp`, alongside `crates/daemon` and `crates/cli`. It is a surface over the same engine,
which is what ADR-0002 says a surface is.

It does **not** link `kuadrat-core`. It speaks to the daemon over loopback HTTP, reusing the same
`curl`-based client shape `crates/cli/src/daemon_client.rs` established — including that module's
proxy-bypass reasoning, which exists so a dead proxy cannot be misread as an absent daemon. `core`
stays untouched by this phase, and ADR-0002's rule that `core` never opens a socket is preserved by
not giving `core` anything new to do.

`kuadrat mcp` is a subcommand on the existing binary rather than a second executable: an MCP client
config names one command, and one binary is one thing for an operator to install and keep in sync.

## Fixed quantities

- **The daemon address** defaults exactly as `kuadrat serve` and `kuadrat deploy` default it, and
  takes the same `--listen` override. Three defaults that must agree are already two too many.
- **`tail_logs`' line count** defaults to 100 and is clamped by `logs::MAX_LINES` — `core` already
  does this clamping and it is not re-implemented here.
- **No timeout of its own.** The daemon's own bounds govern; a second timeout layer would produce a
  tool error for a deploy that is in fact still running.

## Error handling

Three kinds, and they must stay distinguishable:

1. **No daemon** — refuse at startup, name `kuadrat serve`. Never a per-call error, because an agent
   would retry it.
2. **The daemon said no** — a 404 for an unknown app, a 409 for a duplicate deploy. These are the
   daemon's answers and are returned as tool errors carrying its message. `daemon_client.rs` already
   makes the distinction between "no daemon answered" and "the daemon refused"; that distinction is
   the whole reason this maps cleanly.
3. **Malformed request from the client** — JSON-RPC error responses, not process exit. A client that
   sends a bad `tools/call` gets an error and keeps its session.

An unreadable journal reaches the agent as the same message the page and the CLI show, because it
comes from `logs::tail`'s existing detection rather than a second one.

## Testing

The daemon's test harness pattern applies: a `FakeExecutor` and a temp-file store behind a real
router, no socket bound. For this crate the seam is one level up — the tests drive the JSON-RPC loop
over an in-memory pipe against a fake daemon client.

Worth pinning specifically:

- `initialize` before any other method; a `tools/call` arriving first is an error, not a panic.
- `tools/list` matches the implemented tool set. A tool advertised but not dispatchable is the defect
  this test exists to catch.
- An unknown tool name is a JSON-RPC error and the session survives it.
- A 404 from the daemon becomes a tool error, not an empty success — the same "empty read looks like
  a quiet app" failure `core` already guards against, one layer out.
- Startup with no daemon exits non-zero and names `kuadrat serve`.

## Not in this group

- **Authentication and CSRF** — still recorded with their shared trigger. This phase is designed
  specifically not to trip it.
- **`remove` and secret management as tools** — excluded above, with reasons.
- **An HTTP MCP transport** — the thing that would trip the trigger. It becomes reasonable in the
  same change that gives the daemon authentication.
- **The fleet driver** — a remote executor implementing `run_streaming` over SSH, which is why that
  seam's default impl bails rather than being required.

## Open questions

- **Does `deploy` return immediately, or wait for a terminal stage?** Returning immediately keeps the
  tool honest about what it did and lets the agent poll `get_deploy`, but costs the agent a turn.
  Waiting reads better in a transcript and is wrong the moment a deploy takes longer than the client's
  patience. The table above assumes immediate; it should be confirmed against a real agent transcript
  before it is built.
- **Should `reconcile` be a tool at all?** It is the recovery path for a crashed deploy, which is
  exactly when an agent is most likely to be the one looking — and also exactly when a wrong guess
  costs the most.
