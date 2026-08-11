# ADR-0002: A transport-agnostic core crate

- **Date:** 2026-08-10
- **Status:** Accepted

## Context

kuadrat manages one host in v1, but multi-host deployment is the gap competitors leave open
(Cockpit and quadletman are both single-host) and is the most likely direction of growth.

Building multi-host now would triple the work: an agent protocol, node registry, inter-node
auth, and distributed state — before the single-host case is proven. Building single-host
without planning for it usually means a rewrite, because host awareness leaks into every
function signature once one caller needs it.

kuadrat also has four consumers of the same engine: a daemon, a CLI, a web UI, and an MCP
surface for agents.

## Decision

Put the engine in a **library crate, `kuadrat-core`, that never opens a socket and never takes
a host parameter.** Every host interaction goes through a trait. There are **two** such traits,
because a host is touched in two ways:

- **`Executor`** — process execution: every `podman` and `systemctl` invocation.
- **`FileSystem`** — storage: reading, writing, and listing unit files.

Neither takes a host parameter. A transport supplies an implementation of each.

Binaries (`daemon`, `cli`) are thin orchestration over the crate. Everything network-facing —
HTTP, MCP, auth — lives in the daemon.

The invariant, stated so violations are obvious in review:

> If any `kuadrat-core` function grows a `host: &str` parameter, this design has failed.

## Consequences

**What this buys**

- **The fleet driver becomes additive.** A remote transport over SSH is a new `Executor`
  implementation *plus* a new `FileSystem` implementation, not a change to the engine.
  Multi-host stops being a rewrite and becomes a new consumer.

  This originally read "a new `Executor` implementation" alone, and that was false: `Executor`
  covers processes only, so every unit-file write still went to local disk. An `SshExecutor`
  would have written the unit locally and then run `daemon-reload` remotely — not degraded,
  incoherent. Local testing hid it, because `Paths::rooted()` makes the filesystem and the
  process host the same machine, so the seam looks complete precisely where the topology never
  separates the two things it exists to separate. Corrected during the phase-1 whole-branch
  review (finding C3): the `FileSystem` seam closes the gap, and the claim now holds for both
  kinds of side effect.
- **Failure paths become testable.** A fake executor lets tests force a failure at any deploy
  stage and assert the compensation ran. Rollback and crash-recovery are exactly the paths that
  cannot be safely exercised in production and will not be exercised by hand.
- **One implementation for four consumers.** No drift between what the UI does and what the
  agent does, because both call the same functions.
- **The abstraction is not speculative.** The executor seam is introduced for testability,
  which is needed on day one; that it is also the multi-host extension point is a dividend, not
  the justification.

**What this costs**

- **Indirection.** Every `podman` and `systemctl` call goes through a trait rather than
  `Command::new`, and every file operation through a trait rather than `tokio::fs`, which is
  slightly more ceremony to read and write. Engine functions take both seams as parameters.
- **Discipline is required.** The invariant is easy to violate under deadline — one `host`
  parameter "just for this one call" collapses the boundary. Hence stating it as a failure
  condition rather than a preference, and putting it in the plan's global constraints.
- **No direct side effects outside the two local implementations.** A rule reviewers must
  enforce, since nothing in the compiler prevents it. Four clauses, the first two greppable:

  1. `tokio::process::Command` appears only in `exec::local`.
  2. `tokio::fs` appears only in `fs::local` — and neither does `std::fs`, nor
     `Path::exists()`, which is the same violation wearing a different name.
  3. **Exception — the store.** `store` opens its own SQLite file with
     `rusqlite::Connection::open` and creates the containing directory with
     `std::fs::create_dir_all`. This is not a host side effect: the database is
     kuadrat's own state, which stays wherever kuadrat runs, not on the managed
     host a remote executor would reach. It is the one sanctioned direct
     filesystem touch outside `fs::local`.
  4. `EventSink` is the third seam. Publishing an event to a subscriber is a side effect leaving
     `core`, so it goes through a trait rather than a channel type baked into a module — which is what
     keeps `tokio::sync::broadcast` a *daemon* dependency rather than a `core` one. `emit` is
     synchronous and returns nothing: a subscriber that has gone away must not be able to fail a
     deploy, and a sink cannot await, so it cannot suspend the deploy loop; it must also not block —
     a sink that needs I/O hands off to a channel and does that work in its own task.

  Outside `kuadrat-core` the rule does not apply: the CLI reading a spec file off the
  operator's own disk is not a host interaction.

**Rejected alternatives**

*Multi-host from day one* — the real gap, but 3–5× the work, and it would delay proving the
single-host case that everything else depends on.

*Single-host with no abstraction* — simplest to write, but host awareness would spread through
every signature the moment a second host appeared, and the failure paths would stay untested
because there would be no seam to fake.
