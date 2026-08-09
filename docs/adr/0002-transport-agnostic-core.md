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
a host parameter.** Every host interaction goes through an **`Executor` trait**.

Binaries (`daemon`, `cli`) are thin orchestration over the crate. Everything network-facing —
HTTP, MCP, auth — lives in the daemon.

The invariant, stated so violations are obvious in review:

> If any `kuadrat-core` function grows a `host: &str` parameter, this design has failed.

## Consequences

**What this buys**

- **The fleet driver becomes additive.** A remote executor running commands over SSH is a new
  `Executor` implementation, not a change to the engine. Multi-host stops being a rewrite and
  becomes a new consumer.
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
  `Command::new`, which is slightly more ceremony to read and write.
- **Discipline is required.** The invariant is easy to violate under deadline — one `host`
  parameter "just for this one call" collapses the boundary. Hence stating it as a failure
  condition rather than a preference, and putting it in the plan's global constraints.
- **No direct process spawning outside `exec::local`.** A rule reviewers must enforce, since
  nothing in the compiler prevents it.

**Rejected alternatives**

*Multi-host from day one* — the real gap, but 3–5× the work, and it would delay proving the
single-host case that everything else depends on.

*Single-host with no abstraction* — simplest to write, but host awareness would spread through
every signature the moment a second host appeared, and the failure paths would stay untested
because there would be no seam to fake.
