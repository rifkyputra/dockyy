# Known gaps

Carried forward from the phase-1 whole-branch review and its fix wave. Each entry is real, judged
deferrable at the time, and worth re-reading before the phase it names.

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
- **The CLI has no tests of its own.** 68 lines of pure dispatch; add a smoke test when the surface
  grows.
