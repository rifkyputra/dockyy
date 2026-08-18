# kuadrat Phase 6 · Push-to-Deploy and Scheduled Tasks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `git push` to GitHub or GitLab redeploys the app through a verified webhook, and a
spec can declare `tasks` that run on systemd timers in fresh containers from the app's image.

**Architecture:** Scheduled tasks are `core` work: a `tasks` field on the spec, rendered as a
Quadlet oneshot `.container` plus a `.timer` in a new `Paths.systemd_dir`, applied/pruned/removed
with the same ownership rules as the main unit. Push-to-deploy is `daemon` work: two hook routes
on the loopback daemon, HMAC/token verification against `KUADRAT_HOOK_SECRET(_FILE)`, a
git fetch+reset through the `Executor` seam, then the same deploy path the button runs (extracted
so it cannot drift).

**Tech Stack:** Rust 2021, axum, systemd timers, `systemd-analyze calendar`, `sha2` + `hmac`
(the phase's two new dependencies, daemon-only).

**Spec:** [`docs/design/2026-08-18-phase-6-push-and-timers.md`](../design/2026-08-18-phase-6-push-and-timers.md)

## Global Constraints

- **`core` stays socketless** (ADR-0002). Git fetch is network I/O → it lives in `crates/daemon`,
  through the `Executor` seam, like curl does.
- **Two new dependencies, `crates/daemon` only:** `sha2 = "0.10"`, `hmac = "0.12"` (RustCrypto,
  pure Rust). Reason recorded in the design: `std` has no SHA-256, and shelling to `openssl`
  puts the secret on argv. Nothing new anywhere else.
- **Every rendered file carries `# kuadrat-managed: true`** and the `kuadrat-` name prefix;
  `ensure_owned` guards every write and delete, tasks included.
- **Secrets never on argv, never in responses.** The hook secret arrives via env/file; comparisons
  are constant-time (HMAC both sides); 401 bodies never echo anything.
- **A hook route with no secret configured answers 404** — off means absent, same contract as the
  outbound webhook.
- **`make check` must pass**; prefix cargo with `PATH=$HOME/.cargo/bin:$PATH`.
- **Baselines, measured 2026-08-18 at `f2c8143`:** cli **30**, core **202**, daemon **91**,
  mcp **22**.
- Commit after every task with a Conventional Commit subject.

## Fixed quantities, from the design

- Env pair: `KUADRAT_HOOK_SECRET`, else file named by `KUADRAT_HOOK_SECRET_FILE` (trimmed).
- GitHub header `X-Hub-Signature-256`, value `sha256=<hex hmac-sha256 of the raw body>`.
  GitLab header `X-Gitlab-Token`, value = the secret verbatim.
- Payload fields, both forges: branch from `ref` (`refs/heads/<b>`), commit from `after`.
  All-zero `after` = branch deletion = ignored.
- Task unit names: `kuadrat-<app-slug>-task-<task-slug>`; `.container` in `quadlet_dir`,
  `.timer` in `systemd_dir` (default `/etc/systemd/system`, rooted `root/systemd/system`).
- Timer: `OnCalendar=<schedule>`, `Persistent=true`, `WantedBy=timers.target`.
- Schedule preflight: `systemd-analyze calendar <expr>` per task, before any file is written.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/src/spec.rs` | *Modify.* `ScheduledTask`, `tasks` field, validation |
| `crates/core/src/workloads/paths.rs` | *Modify.* `systemd_dir`, task unit names/paths |
| `crates/core/src/workloads/render.rs` | *Modify.* `render_task`, `render_timer` + goldens |
| `crates/core/src/workloads/apply.rs` | *Modify.* Task apply/prune/remove, schedule preflight |
| `crates/daemon/Cargo.toml` | *Modify.* `sha2`, `hmac` |
| `crates/daemon/src/hooks.rs` | *Create.* Secret, verifiers, payload parse, the two routes |
| `crates/daemon/src/api.rs` | *Modify.* Extract `start_deploy`; mount `/hooks/...` routes |
| `crates/daemon/src/lib.rs`, `state.rs` | *Modify.* Load the hook secret at startup into state |
| `README.md`, `docs/known-gaps.md` | *Modify.* Record what landed |

---

### Task 1: `tasks` on the spec

**Files:** Modify: `crates/core/src/spec.rs`

**Interfaces — Produces:**
- `pub struct ScheduledTask { pub name: String, pub schedule: String, pub command: Vec<String> }`
  (derives matching `WorkloadSpec`: Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)
- `pub tasks: Vec<ScheduledTask>` on `WorkloadSpec`, `#[serde(default)]` so every existing
  `kuadrat.json` still parses
- `validate()` extended: per task — name slug non-empty, names unique per spec, `schedule`
  single-line and non-empty, `command` non-empty and every word single-line

- [x] **Step 1: Failing tests** in `spec.rs`'s test module:

```rust
    #[test]
    fn a_spec_without_tasks_still_parses_and_validates() {
        let spec: WorkloadSpec =
            serde_json::from_str(r#"{"name":"web","image":"i"}"#).expect("parse");
        assert!(spec.tasks.is_empty());
    }

    #[test]
    fn duplicate_task_names_are_rejected() {
        let mut spec = WorkloadSpec::new("web", "img");
        for _ in 0..2 {
            spec.tasks.push(ScheduledTask {
                name: "cleanup".into(),
                schedule: "daily".into(),
                command: vec!["true".into()],
            });
        }
        let err = spec.validate().unwrap_err();
        assert!(err.to_string().contains("cleanup"), "{err}");
    }

    #[test]
    fn a_task_schedule_with_a_newline_is_rejected() {
        let mut spec = WorkloadSpec::new("web", "img");
        spec.tasks.push(ScheduledTask {
            name: "t".into(),
            schedule: "daily\nExec=evil".into(),
            command: vec!["true".into()],
        });
        assert!(spec.validate().is_err());
    }

    #[test]
    fn a_task_with_no_command_is_rejected() {
        let mut spec = WorkloadSpec::new("web", "img");
        spec.tasks.push(ScheduledTask {
            name: "t".into(),
            schedule: "daily".into(),
            command: vec![],
        });
        let err = spec.validate().unwrap_err();
        assert!(err.to_string().contains("command"), "{err}");
    }
```

- [x] **Step 2: Run to verify they fail** (no `ScheduledTask`, no field)
- [x] **Step 3: Implement** — the struct; the field; in `validate()`, after the existing checks:
  slug/uniqueness (a `HashSet` over `slug(&t.name)`), `single_line` on name/schedule/each word,
  non-empty checks. Error messages name the task and field, never values (the house rule).
- [x] **Step 4: Suite** — expected core **206** (202 + 4). `make check` clean.
- [x] **Step 5: Commit** — `feat(core): scheduled tasks on the spec`

---

### Task 2: Paths and rendering

**Files:** Modify: `crates/core/src/workloads/paths.rs`, `crates/core/src/workloads/render.rs`

**Interfaces — Produces:**
- `Paths.systemd_dir: PathBuf` (default `/etc/systemd/system`; rooted `root/systemd/system`)
- `pub fn task_unit_name(spec_name: &str, task_name: &str) -> String` →
  `kuadrat-<slug>-task-<taskslug>`
- `pub fn task_container_path(paths, spec_name, task_name) -> PathBuf` (quadlet dir, `.container`)
- `pub fn task_timer_path(paths, spec_name, task_name) -> PathBuf` (systemd dir, `.timer`)
- `pub fn render_task(spec: &WorkloadSpec, task: &ScheduledTask) -> Result<String>` — marker,
  `[Container]` with the spec's Image/Env/Secret + the task's Exec, `[Service] Type=oneshot`
- `pub fn render_timer(spec: &WorkloadSpec, task: &ScheduledTask) -> String` — marker,
  `[Timer] OnCalendar=… Persistent=true`, `[Install] WantedBy=timers.target`

- [x] **Step 1: Failing tests** — golden-file style, matching the module's existing pattern:

```rust
    #[test]
    fn task_units_are_prefixed_and_split_across_the_two_dirs() {
        let paths = Paths::rooted(Path::new("/r"));
        assert_eq!(
            task_container_path(&paths, "My Web", "Daily Cleanup"),
            PathBuf::from("/r/containers/systemd/kuadrat-my-web-task-daily-cleanup.container")
        );
        assert_eq!(
            task_timer_path(&paths, "My Web", "Daily Cleanup"),
            PathBuf::from("/r/systemd/system/kuadrat-my-web-task-daily-cleanup.timer")
        );
    }

    #[test]
    fn renders_a_task_container_and_timer() {
        // golden: crates/core/tests/golden/task.container + task.timer
        // spec with one env, one secret; task {name: "cleanup", schedule: "daily",
        // command: ["sh", "-c", "true"]}
    }

    #[test]
    fn a_task_container_is_oneshot_and_routeless() {
        // rendered text contains "Type=oneshot", no PublishPort, no HealthCmd
    }

    #[test]
    fn rendered_task_files_always_carry_the_managed_marker() { /* both renderers */ }
```

- [x] **Step 2: Run to verify they fail**
- [x] **Step 3: Implement** — reuse `render`'s escaping/quoting helpers (`escape_percent`,
  `quote_word`); write the two golden files by hand first, from the design. The `.timer` needs
  no escaping beyond the marker + validated single-line schedule.
- [x] **Step 4: Suite** — expected core **210** (206 + 4). `make check` clean.
- [x] **Step 5: Commit** — `feat(core): render scheduled tasks as oneshot containers and timers`

---

### Task 3: Apply, prune, remove

**Files:** Modify: `crates/core/src/workloads/apply.rs`

**Interfaces:**
- Consumes: Task 2's paths/renderers, `ensure_owned`, `systemctl`
- Produces: `apply` and `remove` handle tasks; no signature changes

Behavior, in `apply` after the main unit's write and before `daemon-reload`:
1. **Preflight first, before ANY write (including the main unit):** for each task,
   `exec.run("systemd-analyze", ["calendar", <schedule>])`; non-zero → error naming the task.
   This moves to the very top of `apply`, beside `render(spec)?`.
2. Write each task's `.container` and `.timer` (`ensure_owned` both, `create_dir_all` both dirs).
3. **Prune:** `read_dir` both dirs; any file whose stem starts with
   `kuadrat-<slug>-task-` and is not named by the spec → `ensure_owned` then `remove_file`;
   collect pruned timer unit names for `disable --now` after reload.
4. After the existing `daemon-reload`: `systemctl enable --now <timer>` per task;
   `systemctl disable --now <timer>` per pruned timer (ignore failure on never-enabled ones —
   use the existing tolerant pattern if one exists; otherwise a `let _ =` with a comment).

In `remove`: prune ALL of the app's task units (same scan, empty keep-set) before the
existing unit removal; `disable --now` their timers first.

- [x] **Step 1: Failing tests** (FakeExecutor + FakeFileSystem, existing harness style):

```rust
    #[tokio::test]
    async fn apply_writes_task_units_and_enables_their_timers() { /* one task; assert both
        files exist in the fake fs, and calls include ["enable", "--now",
        "kuadrat-web-task-cleanup.timer"] and a systemd-analyze preflight */ }

    #[tokio::test]
    async fn an_invalid_schedule_fails_before_any_file_is_written() { /* script
        systemd-analyze exit 1; assert Err names the task and fake fs is untouched */ }

    #[tokio::test]
    async fn a_task_removed_from_the_spec_is_pruned_on_apply() { /* pre-seed fake fs with a
        marked stale task pair; apply spec without it; both files gone; disable called */ }

    #[tokio::test]
    async fn a_foreign_file_matching_the_task_prefix_is_refused_not_deleted() { /* pre-seed
        an UNMARKED file at a task path; apply must Err (ensure_owned), file intact */ }

    #[tokio::test]
    async fn remove_cleans_up_task_units_with_the_app() { /* seed marked task pair; remove;
        gone; disable --now called before stop */ }
```

- [x] **Step 2: Run to verify they fail**
- [x] **Step 3: Implement** per the behavior list. Keep one `daemon-reload`.
- [x] **Step 4: Suite** — expected core **215** (210 + 5). `make check` clean.
- [x] **Step 5: Commit** — `feat(core): apply, prune, and remove scheduled tasks`

---

### Task 4: The hook verification module

**Files:** Modify: `crates/daemon/Cargo.toml`; Create: `crates/daemon/src/hooks.rs`
(+ `pub mod hooks;` in `lib.rs`)

**Interfaces — Produces:**
- `pub struct HookSecret(String)` with `pub fn from_env() -> Result<Option<HookSecret>>`
  (`KUADRAT_HOOK_SECRET`, else `_FILE`, trimmed — the `webhook.rs` pattern verbatim)
- `pub fn verify_github(secret: &HookSecret, body: &[u8], header: Option<&str>) -> bool` —
  parse `sha256=<hex>`, hex-decode, HMAC-SHA256 the body, compare via double-HMAC
- `pub fn verify_gitlab(secret: &HookSecret, token: Option<&str>) -> bool` — double-HMAC compare
- `pub struct Push { pub branch: String, pub sha: String }`
- `pub fn parse_push(body: &[u8]) -> Option<Push>` — `ref` must be `refs/heads/*`; `after`
  all-zero or missing → `None`

Dependencies (daemon `[dependencies]`):

```toml
# The phase's two new crates, and the only ones: HMAC-SHA256 for webhook
# signatures. std has no SHA-256, and shelling to openssl would put the
# secret on argv — the exact leak the _FILE pattern exists to prevent.
sha2 = "0.10"
hmac = "0.12"
```

- [x] **Step 1: Failing tests** in `hooks.rs`:

```rust
    fn secret() -> HookSecret { HookSecret::for_tests("s3cret") }

    /// Known-answer, computed independently (python3 hmac): pins our parsing
    /// and hex handling, not just crate round-tripping.
    #[test]
    fn a_correctly_signed_github_body_verifies() {
        let body = br#"{"ref":"refs/heads/main","after":"1111111111111111111111111111111111111111"}"#;
        let header = "sha256=a3450400315a03375f96c5ed76f59082b5f4f39ccfd1ad04f1df07ecba18f809";
        assert!(verify_github(&secret(), body, Some(header)));
    }

    #[test]
    fn a_tampered_body_does_not_verify() { /* same header, body + one byte → false */ }

    #[test]
    fn a_missing_or_malformed_header_does_not_verify() { /* None; "sha256=zz"; "md5=..." */ }

    #[test]
    fn the_gitlab_token_verifies_only_on_exact_match() { /* right → true; wrong/None → false */ }

    #[test]
    fn parse_push_reads_branch_and_sha_and_ignores_deletions() {
        /* refs/heads/main + sha → Some; refs/tags/v1 → None; after=0000… → None */
    }
```

`HookSecret::for_tests` is `#[cfg(test)]`-gated so no production path constructs one from a
literal.

- [x] **Step 2: Run to verify they fail**
- [x] **Step 3: Implement** — `Hmac::<Sha256>::new_from_slice`; hex-decode without a new crate
  (a 10-line `fn from_hex`); equality via `hmac(k, a) == hmac(k, b)` on both paths.
- [x] **Step 4: Suite** — expected daemon **96** (91 + 5). `make check` clean.
- [x] **Step 5: Commit** — `feat(daemon): webhook signature verification`

---

### Task 5: The routes, and one deploy path

**Files:** Modify: `crates/daemon/src/api.rs`, `crates/daemon/src/hooks.rs`,
`crates/daemon/src/state.rs`, `crates/daemon/src/lib.rs`

**Interfaces:**
- `api.rs`: the deploy handler's guts become
  `pub(crate) async fn start_deploy(st: &AppState, name: &str) -> Result<i64, ApiError>`
  (busy-check → registration → spec load → validate → reserve → spawn); the `deploy` handler
  and both hook handlers call it. The handler's HTML/JSON branch stays in the handler.
- `state.rs`: `pub hook_secret: Option<Arc<HookSecret>>` on `AppState`; `lib.rs` loads it via
  `HookSecret::from_env()` at startup (a bad `_FILE` path is a startup error, like the webhook).
- `hooks.rs`: axum handlers `github(State, Path(app), HeaderMap, Bytes)` and
  `gitlab(State, Path(app), HeaderMap, Bytes)`; routes mounted in `api.rs`'s router:
  `.route("/hooks/github/:app", post(crate::hooks::github))` and the gitlab twin.

Handler flow (both, differing only in the verify call), exactly the design's ladder:
no secret → 404 · bad signature/token → 401 (empty-ish body: `{"error":"unauthorized"}`) ·
unregistered app → 404 · `parse_push` None → 200 `{"ignored":"not a branch push"}` ·
branch ≠ `git -C <repo> symbolic-ref --short HEAD` (via `st.exec`) → 200 `{"ignored":…}` ·
busy (from `start_deploy`'s 409) → 200 `{"ignored":"deploy in progress"}` ·
`git fetch origin` then `git reset --hard <sha>` (via `st.exec`, in the repo dir; failure →
500, no deploy) · `start_deploy` → 200 `{"deploy_id":N}`.

- [x] **Step 1: Failing tests** (api.rs or hooks.rs test module, existing harness + FakeExecutor;
  the harness env must inject a secret — give `harness_parts` a variant or set the state's
  `hook_secret` directly on the built state):

```rust
    #[tokio::test]
    async fn a_signed_github_push_updates_the_repo_and_deploys() {
        /* register app with repo_path; script git symbolic-ref → "main",
           git fetch → ok, git reset --hard <sha> → ok; POST signed body;
           assert 200 with deploy_id, and calls contain fetch + reset with the
           payload sha */
    }

    #[tokio::test]
    async fn a_bad_signature_is_401_and_runs_no_git() { /* tampered; exec.calls() empty */ }

    #[tokio::test]
    async fn no_secret_configured_means_404_before_any_work() { /* hook_secret: None */ }

    #[tokio::test]
    async fn a_push_to_another_branch_is_ignored_with_a_reason() {
        /* symbolic-ref → "main", payload ref refs/heads/dev → 200 {"ignored":…}, no fetch */
    }

    #[tokio::test]
    async fn a_gitlab_push_with_the_right_token_deploys() { /* X-Gitlab-Token path */ }
```

- [x] **Step 2: Run to verify they fail**
- [x] **Step 3: Implement** — extract `start_deploy` first (a pure refactor; the existing 91
  daemon tests are its net), then the handlers.
- [x] **Step 4: Suite** — expected daemon **101** (96 + 5), everything else unchanged.
  `make check` clean.
- [x] **Step 5: Commit** — `feat(daemon): push-to-deploy hooks for GitHub and GitLab`

---

### Task 6: Record what landed

**Files:** Modify: `README.md`, `docs/known-gaps.md`

- [x] **Step 1: README** — "What it does" gains push-to-deploy + scheduled tasks lines; the
  spec reference gains `tasks`; a "### Push to deploy" section: the env pair, the forge-side
  webhook URL (`https://<your-ingress>/hooks/github/<app>`), and the Caddy exposure block:

```caddy
hooks.example.com {
    reverse_proxy 127.0.0.1:7457
}
```

  with the sentence that the daemon itself stays loopback and the signature is the auth.
- [x] **Step 2: known-gaps** — three entries: no hook queue/debounce (a push during a deploy is
  ignored with a reason; redeliver); Gitea speaks the GitHub shape but is untested and
  unclaimed; task runs report only through systemd/journal (`list-timers` + `tail_logs`), no
  kuadrat-side run history.
- [x] **Step 3: Full gauntlet** — `cargo test --workspace` (expected: cli 30, core 215,
  daemon 101, mcp 22 = **368**), `make check`, no-PreEscaped grep, adblock-bait scan on any
  new DOM (none expected — no UI in this phase).
- [x] **Step 4: Commit** — `docs: record push-to-deploy and scheduled tasks`

---

## Completion checklist

> Closed 2026-08-18, verified on sumo. Measured: cli 30, core 215, daemon **106**, mcp 22 —
> **373 total, 0 failed**. Daemon landed 5 tests over plan: the hook implementation grew four
> hardenings beyond this document — hook git failures reserve a deploy id first so they land as
> durable timeline events; git stderr never reaches a response (it can echo credential-bearing
> remote URLs); a trigger lock serializes check/reset/reserve across concurrent deliveries; and
> `run_git` carries a 30-second server-side deadline. `parse_push` also validates the SHA as
> 40/64 hex. Dependency diff since `f2c8143`: exactly `crates/daemon/Cargo.toml` +5 lines
> (`sha2`, `hmac`, comment) — verified with `git diff f2c8143 -- '**/Cargo.toml'`.

- [x] `cargo test --workspace`: cli 30, core 215, daemon 101, mcp 22 — measured: daemon 106, rest as planned, 0 failed
- [x] `make check` clean
- [x] Two new dependencies, daemon only (`sha2`, `hmac`) — `git diff f2c8143 -- '**/Cargo.toml'` shows nothing else
- [x] A tampered/unsigned hook runs no git command — proven by a test
- [x] No secret configured → hook routes 404 — proven by a test
- [x] The reset uses the payload SHA — proven by a test
- [x] An invalid `OnCalendar` fails before any file is written — proven by a test
- [x] A foreign file at a task path is refused, not deleted — proven by a test
- [x] `remove` cleans up task timers and containers — proven by a test
- [x] The hook path and the button share one `start_deploy`
- [x] Secret never on argv, never in a response body

## Not in this group

Per the design: hook queue/debounce · Bitbucket/Gitea claims · kuadrat-rendered hook ingress ·
per-app secrets · task run history beyond the journal · PR previews, DB backups, fleet driver.
