# Known gaps

Carried forward from the phase-1 whole-branch review and its fix wave. Each entry is real, judged
deferrable at the time, and worth re-reading before the phase it names.

## Open acceptance

**Phase 1's done criterion is formally unmet.** The plan's Task 7 Step 4 — apply a spec on a real
host, confirm the container runs under systemd, then remove it — was never run, because `podman`
is not installed on the development host. Everything below it passed: 41 tests, `make check` clean,
and a temp-root run proving spec parsing, validation, rendering, and the file write.

Two of the Critical findings the whole-branch review caught (C1 unit-file ownership, I1 `Exec=`
quoting) are precisely the class of bug that step would have surfaced first. Run it before trusting
phase 1 on anything real:

```bash
sudo kuadrat apply /path/to/spec.json
sudo systemctl status kuadrat-<slug>
sudo podman ps --filter name=kuadrat-<slug>
kuadrat list && kuadrat status <name>
sudo kuadrat remove <name>
```

## Before phase 2 starts

**`FakeExecutor` scripts output per program, not per argv** (`exec/fake.rs`). Nearly every core call
is `systemctl <verb>`, so all systemctl calls in a test return identical output. This makes
"daemon-reload succeeds but start fails" **inexpressible** — exactly the shape of the per-stage
compensation tests the design calls "the most important layer." Add argv-aware scripting or a
call-sequence queue *before* writing the deploy state machine, not during.

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
