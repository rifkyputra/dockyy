# kuadrat phase 2 — the deploy loop

- **Date:** 2026-08-10
- **Status:** approved design, not yet implemented
- **Builds on:** [phase 1 design](2026-08-10-design.md), which shipped `spec`, `exec`, `fs`, and
  `workloads` and is verified on a real host

## Goal

`kuadrat deploy <app>` takes a local repo to a running service with TLS, and rolls back on failure.

Phase 1 built **Apply** — spec to running unit. Phase 2 builds the other five stages and the machine
that sequences them.

## Scope

### In

- **Deploy state machine** — Detect → Build → Secrets → Apply → Route → Healthcheck, with per-stage
  compensation, a per-app lock, and crash reconciliation
- **Gateway** — one Caddy fragment per app, automatic TLS
- **Secrets** — `podman secret` management; specs carry names, never values
- **Events** — typed, emitted at every transition
- **Store** — SQLite: specs, deploy history, the durable stage, the lock

`WorkloadSpec` gains a `route` field, deferred from phase 1.

### Out

- Cloning from a git URL, and therefore all git credential handling (see Decisions)
- Stack auto-detection beyond a Containerfile — no buildpacks, no generated Containerfiles
- Pull/reconcile loop, webhooks, push-to-deploy
- Blue/green or zero-downtime deploys — in-place replace, as phase 1 decided
- HTTP server, web UI, MCP — those are phases 3 and 4
- Managing Caddy's lifecycle. kuadrat assumes Caddy is installed and running.

## Decisions

| Decision | Choice | Why |
|---|---|---|
| Trigger | Imperative one-shot; pull later as a thin layer | Simplest thing that deploys. Both quadit and quad-ops chose pull; a reconcile loop can call `run()` later, provided it stays idempotent — which this design requires anyway. |
| Source | **Local path only** — no cloning | Removes deploy keys, tokens, host-key verification, and credential storage from phase 2 entirely. The pull layer can own cloning together with the credentials it needs. |
| Build | **Containerfile / Dockerfile only** | Still delivers "repo → running service", which neither competitor does. Generating Containerfiles is its own product. |
| Gateway | One Caddy fragment per app, `import`ed | Mirrors the Quadlet pattern exactly — one file per app, kuadrat-owned, marker-guarded — so the ownership check, drift story, and `FileSystem` seam are reused, not reinvented. |
| Healthcheck | Podman healthcheck, **required for routed apps** | A started unit is not a successful deploy. A spec with a route and no `health_cmd` is rejected: public traffic must not go to something with no readiness signal. |
| Machine shape | `Stage` enum + persisted transitions | Crash reconciliation needs a durable stage regardless, so the enum exists either way; a `match` driver is then the simplest thing that satisfies it. |
| Store | **rusqlite**, synchronous `Store` methods | Single-row lookups on a local file are microseconds. Cheaper than `spawn_blocking` ceremony or sqlx's compile-time and macro weight. |
| Secret input | stdin or `--from-file`, **never argv** | argv is world-readable via `ps`. |

### Prior art considered

[quadit](https://github.com/ubiquitous-factory/quadit) (Rust, pull-only, rootless-only, requires
Podman ≥ 5.4) and [quad-ops](https://github.com/trly/quad-ops) (Go, watches repos of compose files).
Neither builds from source, neither manages a reverse proxy or TLS, and neither has rollback.
Build + gateway + rollback is the unoccupied combination, and kuadrat supports older Podman than
quadit does — the phase-1 acceptance passed on 4.9.3.

quad-ops's premise validates **compose as an input format**; worth considering as a future `Detect`
source, not in v1.

## Architecture

```
deploy/            the state machine — the only module that orchestrates
  ├─ detect        repo path → BuildPlan (Containerfile only)
  ├─ build         podman build, tag = git commit SHA
  └─ machine       Stage enum, driver, compensation, reconciliation
gateway/           Caddy fragment per app + reload
secrets/           podman secret; names in specs, values never
events/            typed Event emitted at every transition
store/             SQLite — specs, deploy history, durable Stage, per-app lock
```

**`deploy` is the only module that orchestrates.** `gateway`, `secrets`, `workloads`, and `store`
know nothing about deploys or about each other. Every stage stays independently testable, and the
pull layer later drives one function rather than five.

Phase 1's invariants hold unchanged: no `host` parameter anywhere in `core`, every host side effect
through `Executor` or `FileSystem`. The new `podman build`, `podman secret`, `git rev-parse`, and
`systemctl reload caddy` calls all go through `Executor`; the Caddy fragment goes through
`FileSystem`.

### The store carve-out

`store` opens a SQLite file directly rather than through `FileSystem`. This is deliberate and is
**not** a seam violation: the database is *kuadrat's own state*, not a side effect on the managed
host. When a remote executor exists, the DB stays wherever kuadrat runs while `Executor` and
`FileSystem` reach the target host. Routing the store through `FileSystem` would make a fleet driver
scatter its own bookkeeping across every managed machine.

**Add this to ADR-0002 as a third clause** so a later reviewer does not "fix" it.

## Components

A `Ctx` bundle, so stages do not take five arguments each:

```rust
pub struct Ctx<'a> {
    pub exec: &'a dyn Executor,
    pub fsys: &'a dyn FileSystem,
    pub store: &'a Store,
    pub paths: &'a Paths,
}
```

| Module | Key interface |
|---|---|
| `deploy` | `run(&Ctx, &DeployRequest) -> Result<DeployOutcome>`, `reconcile(&Ctx) -> Result<Vec<DeployOutcome>>` |
| `deploy::detect` | `detect(path) -> Result<BuildPlan>` |
| `deploy::build` | `build(&Ctx, &BuildPlan, tag) -> Result<String>` → image reference |
| `gateway` | `apply_route(&Ctx, app, &Route)`, `remove_route(&Ctx, app)` |
| `secrets` | `set`, `list`, `remove`, `ensure_all(names)` |
| `events` | `Event { deploy_id, stage, status, detail }` |
| `store` | specs, deploy history, durable `Stage`, per-app lock, event log |

```rust
pub enum Stage { Detect, Build, Secrets, Apply, Route, Healthcheck }

pub enum DeployOutcome {
    Done { image: String },
    RolledBack { failed_at: Stage, cause: String },
    Failed { failed_at: Stage, cause: String },
}

pub struct DeployRequest {
    /// The app's identity. Equal to `WorkloadSpec.name`; its slug names the unit,
    /// the Caddy fragment, and the lock row.
    pub app: String,
    pub path: PathBuf,
}

pub struct Route {
    pub domain: String,
    pub port: u16,
}
```

`Route` is the new `WorkloadSpec` field. A spec with `Some(route)` and no `health_cmd` is rejected
by `validate()`.

### `Executor` gains one method

`podman secret create NAME -` takes the value on **stdin**, and the phase-1 trait is
`run(program, args)` only. Passing a secret in argv would expose it in `ps` output to every user on
the box.

```rust
async fn run_with_stdin(&self, program: &str, args: &[String], stdin: &str)
    -> Result<CommandOutput>;
```

Default-implemented to return an error, so `LocalExecutor` and `FakeExecutor` opt in without
breaking phase-1 callers.

**`FakeExecutor` must record the call but never the stdin.** A test double that logs secret values
turns every CI run into a leak.

### Paths

`Store` lives at `/var/lib/kuadrat/kuadrat.db` and Caddy fragments at
`/etc/caddy/kuadrat.d/<slug>.caddy`, both injectable through `Paths` so tests use a temp root. The
operator's Caddyfile needs one line: `import kuadrat.d/*.caddy`.

## Data flow

```
acquire lock (per app) ─ reject if held
  └─ insert deploy row → deploy_id, stage = Detect
       │
       ├─ Detect      → BuildPlan          ┐
       ├─ Build       → image:<sha>        │ after each: persist stage,
       ├─ Secrets     → verified           │ emit Event
       ├─ Apply       → unit written       │
       ├─ Route       → fragment + reload  │
       └─ Healthcheck → healthy            ┘
       │
  mark Done, store spec as current ── release lock
```

Every transition writes the stage to SQLite **before** the next stage runs. That row is the
crash-recovery mechanism, not a log of one.

| Stage | Does | Fails when |
|---|---|---|
| Detect | Find a Containerfile or Dockerfile, read `git rev-parse HEAD` for the tag | Neither file present, or the path is not a git repo |
| Build | `podman build`, tagged with the commit SHA | Build error |
| Secrets | Verify every name the spec references exists in `podman secret` | A referenced name is missing |
| Apply | Render the unit, compare with disk, write, reload, start or restart | Invalid unit, start failure |
| Route | Write the Caddy fragment, `systemctl reload caddy` | Caddy rejects config, ACME failure |
| Healthcheck | Poll podman health until healthy or 60s elapse | Timeout, unhealthy |

Ordering is a correctness decision. Secrets are verified **before** Apply so a missing credential
fails while the old version is still serving. Route follows Apply so traffic is never sent to a
service that is not up. Healthcheck gates `Done`.

### Apply must restart, not start, when the unit changed

Phase 1 always runs `systemctl start`, which is a **no-op on an already-running unit**. A redeploy
with a new image would write a new unit file while the old container kept running — a deploy that
reports success and changes nothing.

Apply compares the rendered unit against what is on disk: identical → `start` (no-op if running);
different → `daemon-reload` then `restart`.

This also gives idempotence for free: same path, same commit, same spec → byte-identical unit →
nothing restarts.

**A dirty working tree produces the same tag as a clean one at that commit.** Detect reads
`git rev-parse HEAD` and does not inspect `git status`, so uncommitted edits are built into an image
tagged as if they were the commit. Acceptable for a single-host tool where the operator controls the
checkout; the fix, when it bites, is appending a short content digest for a dirty tree. Do not
"solve" it by refusing to deploy a dirty tree — deploying a work-in-progress checkout is a normal
thing to want on your own server.

### Compensation

| Failed at | Undo |
|---|---|
| Detect, Build, Secrets | Nothing — the host is untouched, old version still serving |
| Apply | Re-apply the previous spec, or remove the unit if there was no previous |
| Route | Remove or restore the fragment, reload Caddy, then unwind Apply |
| Healthcheck | Unwind Route, then Apply |

Rollback re-applies the previous spec from SQLite through the same Apply → Route → Healthcheck path.
No special-case code, and rollback is covered by the same tests as a normal deploy.

### The lock must survive a crash

If kuadrat dies mid-deploy, the lock row stays held and every future deploy of that app is rejected
forever. Reconciliation owns this: for each `in_progress` deploy, finish or roll back, then release
the lock. The lock is released on **every** exit path — success, rollback, and reconciliation.

### Reconciliation

On startup, compare the running unit against the last `Done` spec and either complete the deploy or
unwind it, using the same compensation table. No separate recovery code path.

## Error handling

One typed error, tagged with its stage — this is where `thiserror`, declared but unused since phase
1, earns its place:

```rust
#[derive(Debug, thiserror::Error)]
#[error("deploy failed at {stage:?}: {kind}")]
pub struct DeployError {
    pub stage: Stage,
    pub kind: ErrorKind,
    #[source] pub source: Option<anyhow::Error>,
}
```

The stage tag is what makes phase 4's agent diagnosis tractable — it receives "failed at Healthcheck
after 60s, unit active, health status unhealthy" rather than a log dump. It is also what the phase-3
web UI renders as a progress view.

| Outcome | Means |
|---|---|
| `Done` | Healthcheck passed. The only success. |
| `RolledBack` | A stage failed **and** compensation succeeded. The old version is serving. |
| `Failed` | A stage failed **and compensation also failed.** Host state is unknown. |

`Failed` never auto-retries. kuadrat stops, emits the event, and leaves the host exactly as-is. A
compensation that just failed is not more likely to work a second time, and retrying can compound
the damage. A human or the agent decides.

Four specific rules:

- **Healthcheck timeout is 60s, a constant.** Add a spec field when a real service needs longer.
- **Secrets never reach an error message.** Values travel by stdin so podman will not echo them; the
  rule binds us — no `.context()` may interpolate a value. The Secrets stage reports missing *names*.
- **A rejected concurrent deploy is not an error state.** Lock held → return immediately with a clear
  message. No deploy row, no events, no host changes.
- **Reconciliation failures do not block startup.** Mark the stale deploy `Failed`, log, carry on —
  one broken app must not stop the daemon managing the others.

## Testing

1. **Detect** — pure, against repo fixtures in `tests/fixtures/`: has a Containerfile, has a
   Dockerfile, has neither (rejected with a useful message).
2. **Golden tests for the Caddy fragment** — same pattern and same reasoning as the unit-file
   goldens: a malformed fragment breaks Caddy for every site on the box, not just this one.
3. **Store** — against a temp-root DB. Lock acquire/reject/release, stage persistence, previous-spec
   lookup, event append.
4. **The compensation matrix** — fake `Executor` + fake `FileSystem` + temp DB, failure forced at
   each of the six stages, asserting the right undo ran. **This is the layer that matters**, and it
   only became expressible when `FakeExecutor::expect_call` landed.
5. **Reconciliation** — seed a DB with an `in_progress` deploy at each stage, run `reconcile`, assert
   the outcome **and that the lock was released**. A passing reconcile that leaks the lock bricks the
   app permanently.
6. **Real-host acceptance** — extend `scripts/acceptance.sh`: deploy a fixture repo, verify the route
   serves over TLS, deploy a broken commit, verify rollback left the old version serving.

Two tests worth naming because both cover things that would otherwise pass silently:

- **Redeploy with a changed image issues `restart`, not `start`** — assert on the call sequence.
- **A secret value never appears in `FakeExecutor::calls()`.**

Not tested: podman's own build behaviour and Caddy's TLS issuance. Both belong to the acceptance
script on a real host.

**Gate:** `make check && make test` with zero warnings, unchanged from phase 1.

## Implementation sequence

Too large for one plan. Five **task groups** — numbered independently of the project's phases, which
this document calls phase 1 through 4. Each group gets its own implementation plan and is
independently useful:

| Group | Contents | Done when |
|---|---|---|
| **G1** | Store + events — rusqlite schema, specs, deploy history, durable stage, per-app lock, event append | A deploy row can be created, advanced, and locked against |
| **G2** | Detect + Build — Containerfile discovery, `git rev-parse`, `podman build` | A repo path yields a tagged image |
| **G3** | Gateway + secrets — Caddy fragment with ownership guard, `run_with_stdin`, podman secret CRUD | A route serves, and a secret round-trips without appearing in argv or the call log |
| **G4** | The state machine — driver, compensation matrix, restart-on-change, lock lifecycle | `kuadrat deploy` works end to end and rolls back on a forced failure |
| **G5** | Reconciliation + acceptance — startup recovery, extended acceptance script | A deploy killed mid-flight resolves cleanly on restart |

**G4 is the wedge.** G1–G3 are its prerequisites; G5 is what makes it trustworthy.

## Carried in from `known-gaps.md`

Closed as part of this work:

- **Orphan unit file on failed reload** — the Apply compensation handles it, built on the existing
  ownership check rather than a second one.
- **`thiserror` unused** — `DeployError` uses it.
- **Slug collisions** — the store rejects a second app whose slug collides with an existing one.
- **`%` specifier escaping** — closed with secrets, as the note predicted: `validate()` gains `%`
  handling for `Exec=` and `Environment=`.
- **`Paths` reachable by two public paths** and **no crate-root API surface** — fold into task 1,
  once the phase-2 surface is known.
- **Validation boundary is apply-only** — `remove` and `status` validate too.
