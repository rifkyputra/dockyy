# Known gaps

Carried forward from the phase-1 whole-branch review and its fix wave. Each entry is real, judged
deferrable at the time, and worth re-reading before the phase it names.

## From H1 — `reconcile` emits no events

**Narrowed 2026-08-11 in H5.** `reconcile` now appends the deploy-level terminal event for each row
it rolls back or fails, so a crash-recovered deploy has an explicit ending in its timeline and its
stream closes instead of hanging. Nobody is subscribed while reconcile runs — it completes before
the listener binds — so the value is entirely in the stored record that `/deploy/:id` renders
afterwards.

Still open: reconcile emits no *stage* events, so the UI shows the ending without showing that a
crash recovery produced it. The terminal event's `detail` carries the cause, which is the part that
matters; a dedicated "recovered by reconcile" signal can wait for a group that has a UI to show it.

## From H1 — no terminal event

**Closed 2026-08-11 in H5.** `EventKind::Finished { status: DeployStatus }` is a stored event, so
the `deploys` table and the event log now agree about where a deploy's story ends, and a rollback
that succeeded is visible rather than inferred from silence. It is stored as the literal `"deploy"`
in the `events.stage` column with the `DeployStatus` in `status` — no schema change, and rows
written before H5 read back unaltered. The SSE handler closes on that event.

Of the two options recorded here, the second was taken. Polling the `deploys` row would have closed
the stream but left the successful-rollback case invisible, which was half the complaint.

One path still finishes a deploy without an event: `reserve` rejecting a duplicate writes `Failed`
and returns `Err`, so the caller gets a 409 and is never handed the id. The stream covers that row
by checking its status before deciding to wait.

## From G5 — a per-row store error aborts the whole reconcile batch

`deploy::reconcile` `?`-propagates store errors (`load_previous`/`finish_deploy`/`release_lock`) per
row. If one `in_progress` deploy has, say, a corrupt stored spec JSON, reconcile returns `Err` and
leaves the remaining `in_progress` rows unreconciled and that row's lock still held. It self-heals on
the next startup run (reconcile is startup-idempotent), so it was deferred. A future pass could
collect per-row errors into the returned outcomes and continue the batch instead of aborting.

## From G4b — a `release_lock` DB error drops the terminal outcome

`deploy::run` propagates `release_lock(&name)?` after `run_stages`. If the SQLite `DELETE` fails
(disk full, corruption), `run` returns that error and the real `Done`/`RolledBack`/`Failed` outcome
is dropped, and the lock stays held until G5's reconciliation releases it. A local `DELETE` failing
is rare, and reconciliation recovers a stuck lock, so this was deferred. A clean fix wants a way to
surface the real outcome while still signalling the release failure — worth a look once core has any
logging/telemetry, or fold into G5 where reconciliation already owns stuck locks.

## From H2 — CLI-deployed apps have no registration

H2 added `app_config` (registered apps) alongside `apps` (deployed apps), but nothing back-fills
`app_config` for an app that was already deployed from the CLI before registration existed, or that
is deployed straight from argv without ever calling `register_app`. On the acceptance host, every
app deployed there came from the CLI, so each has an `apps` row and no `app_config` row.

Nothing is broken and no data is lost — `current_spec`, `apply`, and the rest of the deploy path
read `apps`, not `app_config`, and none of that changes. This is a display consequence, not a data
one: a UI app list built by calling `list_app_configs` would come back empty on a host that is
actively running several apps, and "zero apps" is exactly what data loss would also look like to the
operator looking at the screen, even though it isn't.

Two options for the next group:

1. List the union of `apps` and `app_config`, keyed by name.
2. Have `kuadrat deploy` back-fill `register_app` with its argv repo path on every run, so the two
   tables converge on their own.

Option 2 is preferred. It is smaller — the UI needs no union/merge logic, `list_app_configs` stays
the one source the app list reads from. It repairs the host incrementally, after one deploy per app,
with no migration step. And it makes the CLI and the UI agree on what "an app" is going forward,
rather than leaving `apps` and `app_config` as two answers to the same question indefinitely.

## Acceptance — PASSED 2026-08-10

Phase 1's done criterion is met. `scripts/acceptance.sh` ran on a real host (Ubuntu 24.04.4,
Podman 4.9.3, cgroups v2) and passed 16/16: apply wrote a correct unit, systemd reported it active,
podman showed the container, `list`/`status` agreed with reality, and remove cleaned up both.

It also regression-tested the two Critical findings against a real host, which is where they would
actually bite: **C1** — kuadrat refused both to overwrite and to delete a planted foreign
`.container` file; **C2** — a spec with `1\nUser=root` in an env value was rejected rather than
rendered; **I1** — `sh -c "echo …; sleep 3600"` rendered as one quoted argument, not four.

Re-run it after any change to rendering, paths, or the ownership guard:

```bash
cd ~/devbox/kuadrat && PATH=$HOME/.cargo/bin:$PATH cargo build --release
sudo bash scripts/acceptance.sh
```

## Before phase 2 starts

~~**`FakeExecutor` scripts output per program, not per argv.**~~ **Closed 2026-08-10.**
`FakeExecutor::expect_call(program, args, output)` matches an exact `(program, args)` pair and takes
precedence over the program-wide `expect()`, which still works — so existing tests were untouched.
`apply_fails_at_start_after_a_successful_reload` (`workloads/apply.rs`) is the previously
inexpressible case, now covered: the reload succeeds, the start fails, and the test asserts on both
the error and the call sequence. Phase 2's per-stage compensation tests can be written directly.

**`apply()` writes the unit before `daemon-reload` succeeds**, so a failed reload leaves an orphan
file. Acceptable today because units are derived artifacts and the ownership guard means the next
apply overwrites it — but the deploy state machine's per-stage compensation must handle it, and
should be built on the same ownership check rather than a second one.

## G3 — secrets

- **`secrets::set` upserts via remove-then-create, not an atomic replace.** podman 4.9.3's
  `secret create --replace NAME -` is broken for a not-yet-existing secret (it errors trying to
  delete a nonexistent old one), so `set` does a best-effort `podman secret rm` followed by
  `podman secret create`. There is a window between the two where the secret is absent — and, if
  `create` fails after the `rm`, permanently until re-set; a subsequent `ensure_all` gate catches
  the missing name so a deploy fails safe rather than serving half-state. Acceptable for a
  single-host tool: a container reads its mounted secret at start, not while `kuadrat secret set`
  is running, so the window does not race a live read. Revisit if kuadrat ever needs to rotate a
  secret under a running container without a restart.

## Before G4

- **`render` does not know the built image reference.** It emits `Image={spec.image}` straight
  from the spec's free-form `image` field; it never calls `image_reference`. So G4's Apply stage
  must set `spec.image = image_reference(slug, plan.commit)` itself, and must derive the spec's
  name/slug from the same identity the build used — otherwise the image tag and the container
  namespace can drift apart. Recorded here so G4 does not assume `render` already knows the built
  reference.

## Injection family (same root as C2)

`WorkloadSpec::validate()` rejects `\n` and `\r` in every rendered field, closing directive
injection. It does **not** escape `%`. Quadlet copies `Exec=` into the generated `ExecStart=`, where
systemd expands specifiers (`%H`, `%i`, …); the same applies to `Environment=`. A literal `%` needs
`%%`. Pre-existing, narrower than C2, and unchanged by the fix wave — but it is the residual of the
same family and should be closed when secrets handling lands.

## Smaller items

- **Slug collisions.** `"My App"`, `"my_app"`, and `"my-app"` all slug to `my-app`, so two distinct
  specs silently target the same unit. Deferred to a phase-2 registry that can reject a duplicate.
- **Validation boundary is `apply`-only.** `remove` and `status` skip `validate()`, so an
  empty-slug name reaches `unit_path` as `kuadrat-.container`. Harmless — the file never exists, so
  no `systemctl` call is made — but the asymmetry is worth removing.
- **ADR-0002's reviewer rule is stated too absolutely.** Clause 2 says `std::fs` and
  `Path::exists()` do not appear in the crate; both legitimately appear in `#[cfg(test)]` blocks.
  A literal grep-enforcement fires false positives. One sentence to fix.
- **`FakeFileSystem::read_dir` returns files only, never subdirectories.** Irrelevant to the
  `.container` scan today; would mask a future bug about directory entries.
- **`Paths` is reachable by two public paths** — `workloads::apply::Paths` (a re-export) and
  `workloads::paths::Paths`. Consumers are split between them. Pick one.
- **No crate-root API surface.** `lib.rs` re-exports nothing, so consumers write
  `kuadrat_core::workloads::apply::apply`. Add root re-exports while there is still one consumer.
- **`thiserror` is declared but unused.** Either land the design's stage-tagged error enum in phase
  2 or drop the dependency.
- ~~**The CLI has no tests of its own.**~~ **Closed 2026-08-11.** The surface grew as predicted, and
  two arms had stopped being pure dispatch: `--route` parsing and the app name `build` derives from
  a repo path. Both moved to `crates/cli/src/args.rs` (`parse_route`, `app_name`) with 13 tests, and
  the `match` arms call them. The extraction closed three holes the inline versions had: `:3000`
  parsed as an empty domain, `https://example.com:3000` parsed the scheme as part of the domain, and
  `:0` parsed as port 0 — each of which would have rendered a Caddy fragment that loads but serves
  nothing. `app_name` now also rejects a directory whose name slugs to empty, which would have
  produced the image tag `localhost/kuadrat-:<sha>`. The remaining arms are still pure dispatch and
  are covered by the acceptance scripts.

## From H3 — journald content is not sanitised

`logs::tail` and `logs::search` (when it arrives) return whatever the application wrote to its
stdout and stderr. A workload that logs a token or a password will have that value returned by
this module and, from H4 onward, rendered in the web UI and served over the API. kuadrat's own
code never writes a secret to a log — the secrets stage reports names only — but it cannot vouch
for what a deployed application logs.

This is why the daemon binds loopback and ships no authentication: log content is as sensitive as
the least careful app on the host. Log lines also pass through lossy UTF-8 conversion and may
carry ANSI escapes, control characters, and HTML such as `<script>`; escaping belongs at the
render boundary in the web UI, not in this module, because escaping here would corrupt the JSON
consumer.

## From H3 — the privilege signal trusts an inherited environment variable

`logs::journal_unreadable` tells "this app is quiet" apart from "this process cannot read the
journal" by matching journald's stderr hint. That hint is emitted through systemd's logging,
which honours `$SYSTEMD_LOG_LEVEL` from the process's environment, and `LocalExecutor` runs
`Command::output()` inheriting whatever environment the daemon started with. A daemon started
with `SYSTEMD_LOG_LEVEL=warning` or higher gets journalctl's hint suppressed exactly as `-q`
would have — empty stderr, exit 0 — and `logs::tail`/`logs::search` silently return `Ok(vec![])`
for a journal they were never able to read, indistinguishable from a genuinely quiet unit.

`Executor` has no environment parameter today, so this cannot be closed inside `logs`. Two
things carry forward: operationally, `SYSTEMD_LOG_LEVEL` must not be set to `warning` or above
for the daemon's process; and structurally, pinning `SYSTEMD_LOG_LEVEL=info` and `LC_ALL=C` for
the journalctl child specifically belongs with a future `Executor` env parameter, not a
workaround in this module.
