# kuadrat Phase 3 · H7 — Serve Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** An operator installs the unit, starts the service, and deploys from either the CLI or the
browser. When a deploy ends — or a stage fails — a message arrives in their chat.

**Architecture:** One new daemon module, `webhook.rs`, holding the sender and the task that
subscribes to the broadcast hub. Two CLI changes: a `serve` subcommand, and a `deploy` that tries
the daemon before running in-process. A systemd unit and an acceptance script that exercises the
whole surface on a real host.

**Tech Stack:** Rust 2021, axum 0.7, `curl` through the existing `Executor` seam, systemd.

**Design:** [`docs/design/2026-08-11-phase-3-h7-serve.md`](../design/2026-08-11-phase-3-h7-serve.md),
refining [`2026-08-11-phase-3-daemon-and-surfaces.md`](../design/2026-08-11-phase-3-daemon-and-surfaces.md).

## Global Constraints

- **No new dependencies, anywhere.** The webhook and the CLI's daemon probe both go through `curl`
  via `Executor`, which is how this codebase already reaches the host. `crates/cli` gains a path
  dependency on `kuadrat-daemon` — that is the stated direction (`cli → daemon → core`), not a new
  third-party crate.
- **`core` never opens a socket and never takes a `host` parameter** (ADR-0002). Nothing in
  `crates/core` changes in this group.
- **The webhook URL is a secret.** It reaches `curl` through `Executor::run_with_stdin` as a
  `curl --config -` document and **never as an argv element** — argv is world-readable through `ps`.
  It is never logged in full. `secrets::set` established both the seam and the reasoning; follow it.
- **A webhook can never affect a deploy.** Failures are logged and dropped after the bounded retry.
- **"Cannot reach the daemon" and "the daemon said no" must stay distinguishable.** A 4xx is the
  daemon's answer and is reported; only a connection failure falls back to running locally.
- **`make check` must pass**: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
- **Prefix cargo commands with `PATH=$HOME/.cargo/bin:$PATH`.**
- **Baselines, measured at `54b6886`:** `kuadrat` (cli) **17**, `kuadrat_core` **187**,
  `kuadrat_daemon` **65**.
- Commit after every task with a Conventional Commit subject.

## What cannot be verified here

`scripts/serve-acceptance.sh` needs root — it installs a unit, runs `systemctl daemon-reload`, and
deploys a real workload. The five existing acceptance scripts are all run with `sudo` by the
operator, not by the build.

So Task 8 **writes** the script and Task 9 documents it, but neither runs it. Say so in the report
rather than implying it passed. The unit tests prove the pieces; the script proves the whole on a
host, and that run is Rifky's to make.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/daemon/src/webhook.rs` | *Create.* URL loading, event selection, payload, `curl` delivery with retry, the subscriber task |
| `crates/daemon/src/lib.rs` | *Modify.* Declare the module, spawn the subscriber, drop `Config.socket` |
| `crates/daemon/src/config.rs` | *Modify.* Remove the dead `socket` field |
| `crates/daemon/examples/serve.rs` | *Delete.* Superseded by `kuadrat serve` |
| `crates/cli/Cargo.toml` | *Modify.* Depend on `kuadrat-daemon` |
| `crates/cli/src/main.rs` | *Modify.* `serve`; `deploy` tries the daemon first |
| `crates/cli/src/daemon_client.rs` | *Create.* The `curl`-based probe and deploy call, and its outcome type |
| `packaging/kuadrat.service` | *Create.* The unit |
| `scripts/serve-acceptance.sh` | *Create.* The sixth acceptance script |

---

### Task 1: The webhook's pure parts — URL, selection, payload

**Files:**
- Create: `crates/daemon/src/webhook.rs`
- Modify: `crates/daemon/src/lib.rs` (declare `pub mod webhook;`)

**Interfaces:**
- Consumes: `StoredEvent`, `EventKind`, `EventStatus`, `DeployStatus`
- Produces:
  - `pub struct Webhook { url: String }` with `pub fn from_env() -> Result<Option<Webhook>>` and
    `pub fn new(url: String) -> Webhook`
  - `pub fn is_notable(ev: &StoredEvent) -> bool`
  - `pub fn payload(app: &str, ev: &StoredEvent) -> String` — the JSON body
  - `pub fn curl_config(url: &str, body: &str) -> String`

No I/O in this task beyond reading an env var and a file. Delivery is Task 2.

- [ ] **Step 1: Write the failing tests**

Create `crates/daemon/src/webhook.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kuadrat_core::deploy::{DeployStatus, Stage};
    use kuadrat_core::events::{Event, EventStatus};

    fn stored(id: i64, kind_of: &str) -> StoredEvent {
        let event = match kind_of {
            "finished" => Event::finished(12, DeployStatus::RolledBack, Some("apply broke".into())),
            "failed" => Event::for_stage(12, Stage::Apply, EventStatus::Failed, Some("apply broke".into())),
            "started" => Event::for_stage(12, Stage::Apply, EventStatus::Started, None),
            _ => Event::for_stage(12, Stage::Apply, EventStatus::Succeeded, None),
        };
        StoredEvent { id, at: "2026-01-01 00:00:00".into(), event }
    }

    /// The receiver wants warnings, not a trace. One to three messages per
    /// deploy is the design's estimate, and it only holds if the ordinary
    /// stage traffic is filtered out here.
    #[test]
    fn only_endings_and_failures_are_notable() {
        assert!(is_notable(&stored(1, "finished")));
        assert!(is_notable(&stored(2, "failed")));
        assert!(!is_notable(&stored(3, "started")));
        assert!(!is_notable(&stored(4, "succeeded")));
    }

    #[test]
    fn the_payload_carries_a_readable_line_and_the_structured_fields() {
        let body = payload("web", &stored(9, "finished"));
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");

        assert_eq!(v["app"], "web");
        assert_eq!(v["deploy_id"], 12);
        assert_eq!(v["stage"], "deploy");
        assert_eq!(v["status"], "rolled_back");
        assert_eq!(v["detail"], "apply broke");

        let text = v["text"].as_str().expect("text");
        assert!(text.contains("web"), "the app must be readable in the line: {text}");
        assert!(text.contains("12"), "the deploy id must be readable: {text}");
    }

    /// `stage` and `status` come from the same projection the database and the
    /// JSON API use, so a deploy-level event spells it "deploy" on all three
    /// surfaces and they cannot drift.
    #[test]
    fn a_stage_failure_names_its_stage_not_deploy() {
        let body = payload("web", &stored(9, "failed"));
        let v: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(v["stage"], "apply");
        assert_eq!(v["status"], "failed");
    }

    /// The whole point of the config document: the URL carries a token, and
    /// argv is world-readable through `ps`.
    #[test]
    fn the_curl_config_carries_the_url_and_the_body() {
        let cfg = curl_config("https://example.com/hook/TOKEN", r#"{"text":"hi"}"#);
        assert!(cfg.contains(r#"url = "https://example.com/hook/TOKEN""#), "{cfg}");
        assert!(cfg.contains("Content-Type: application/json"), "{cfg}");
        assert!(cfg.contains(r#"\"text\":\"hi\""#), "the body must be escaped for the config: {cfg}");
    }

    /// A quote or a backslash in the body must not end the config value early
    /// — that would truncate the request or, worse, let a log line inject a
    /// curl option.
    #[test]
    fn a_body_containing_quotes_and_backslashes_is_escaped() {
        let cfg = curl_config("https://example.com/h", r#"{"detail":"say \"hi\" C:\\x"}"#);
        for line in cfg.lines() {
            if let Some(rest) = line.strip_prefix("data = ") {
                assert!(rest.starts_with('"') && rest.ends_with('"'), "unquoted: {rest}");
                // Every interior quote must be escaped.
                let inner = &rest[1..rest.len() - 1];
                assert!(
                    !inner.contains(r#"""#) || inner.contains(r#"\""#),
                    "an unescaped quote ends the value early: {inner}"
                );
            }
        }
    }

    #[test]
    fn no_configuration_means_no_webhook() {
        // Both variables absent.
        std::env::remove_var("KUADRAT_WEBHOOK_URL");
        std::env::remove_var("KUADRAT_WEBHOOK_URL_FILE");
        assert!(Webhook::from_env().expect("read").is_none());
    }
}
```

Note the env test mutates process-global state. Keep it to the one test, and do not add a second
env-reading test in this module — two of them running concurrently under the test harness would
race. If you need more coverage of the file path, test a helper that takes the two values as
arguments rather than reading the environment.

- [ ] **Step 2: Run to verify they fail**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test -p kuadrat-daemon webhook:: 2>&1 | tail -20
```

Expected: compile failure — nothing is defined yet. Declare `pub mod webhook;` in `lib.rs` first or
the module is not compiled and the run reports zero tests.

- [ ] **Step 3: Implement**

```rust
//! The outbound webhook — the daemon's doorbell.
//!
//! Lives here and not in `core` because `crates/daemon` is, as its own module
//! doc says, the only networked code in kuadrat. It reaches the network the
//! same way everything else reaches the host: through the `Executor` seam,
//! shelling out to `curl`. That buys no new dependency and a sender that is
//! testable with `FakeExecutor` rather than a fake HTTP server.

/// Where to POST. Absent configuration means the sender is off — that is not
/// an error and must not warn on every start.
pub struct Webhook {
    url: String,
}

impl Webhook {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    /// `KUADRAT_WEBHOOK_URL`, else the contents of the file named by
    /// `KUADRAT_WEBHOOK_URL_FILE`.
    ///
    /// A file is offered because a URL carrying a token is a secret, and a
    /// systemd unit's `Environment=` line is readable by anyone who can run
    /// `systemctl show`. `LoadCredential=` and a file keep it out of that.
    pub fn from_env() -> Result<Option<Self>> { .. }
}

/// Whether this event is worth a message.
///
/// Terminal outcomes and stage failures only. The receiver wants warnings, not
/// a trace: a deploy emits thirteen events and at most three of them belong in
/// someone's chat.
pub fn is_notable(ev: &StoredEvent) -> bool {
    matches!(
        ev.event.kind,
        EventKind::Finished { .. }
            | EventKind::Stage {
                status: EventStatus::Failed,
                ..
            }
    )
}
```

`payload` builds the JSON with `serde_json::json!`, taking `stage` and `status` from
`ev.event.kind.columns()` — the same projection the store writes and the API returns.

`curl_config` emits three lines, `url`, `header`, and `data`, each value quoted, with `\` and `"`
escaped inside the value. Write the escape as a small function; it is the part that has a test
because it is the part that can be wrong.

- [ ] **Step 4: Run the suite**

Expected: daemon **71** (65 + 6), core 187, cli 17. `make check` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): the webhook's payload, selection, and curl config"
```

---

### Task 2: Delivery, with a bounded retry

**Files:**
- Modify: `crates/daemon/src/webhook.rs`

**Interfaces:**
- Consumes: `Executor::run_with_stdin`, `FakeExecutor` for tests
- Produces: `pub async fn send(exec: &dyn Executor, hook: &Webhook, body: &str) -> Result<()>`

- [ ] **Step 1: Write the failing tests**

```rust
    #[tokio::test]
    async fn a_successful_post_runs_curl_once_with_the_url_on_stdin() {
        let exec = FakeExecutor::new();
        exec.expect("curl", out(0, "", ""));

        send(&exec, &Webhook::new("https://example.com/h/TOKEN".into()), r#"{"text":"x"}"#)
            .await
            .expect("send");

        let calls = exec.calls();
        assert_eq!(calls.len(), 1);
        let (program, args) = &calls[0];
        assert_eq!(program, "curl");
        assert!(
            !args.iter().any(|a| a.contains("TOKEN")),
            "the token must never reach argv: {args:?}"
        );
        assert!(args.iter().any(|a| a == "--config"), "{args:?}");
    }

    /// Best-effort with a bounded retry: three attempts, then give up. The
    /// deploy is long finished by then and nothing is waiting on this.
    #[tokio::test]
    async fn a_failing_post_is_retried_three_times_and_then_dropped() {
        let exec = FakeExecutor::new();
        exec.expect("curl", out(7, "", "could not connect"));

        let result = send(&exec, &Webhook::new("https://example.com/h".into()), "{}").await;

        assert!(result.is_err(), "the caller is told, even though it will only log it");
        assert_eq!(exec.calls().len(), 3, "three attempts, not more and not fewer");
    }

    /// An HTTP error is a failure like any other here — the doorbell did not
    /// ring. `--fail` is what makes curl report a 4xx as a non-zero exit.
    #[tokio::test]
    async fn curl_is_asked_to_treat_an_http_error_as_a_failure() {
        let exec = FakeExecutor::new();
        exec.expect("curl", out(0, "", ""));
        send(&exec, &Webhook::new("https://example.com/h".into()), "{}")
            .await
            .expect("send");
        let (_, args) = &exec.calls()[0];
        assert!(args.iter().any(|a| a == "--fail"), "{args:?}");
    }
```

`FakeExecutor` records both sides: `calls() -> Vec<(String, Vec<String>)>` and
`stdins() -> Vec<String>` (`crates/core/src/exec/fake.rs:58,90`, kept deliberately separate). So
assert **both** halves of the secret-handling property, not just one:

```rust
        assert!(
            !args.iter().any(|a| a.contains("TOKEN")),
            "the token must never reach argv: {args:?}"
        );
        assert!(
            exec.stdins()[0].contains("TOKEN"),
            "the URL must arrive on stdin instead"
        );
```

An absence assertion on its own would also pass if the URL never reached curl at all — the presence
assertion is what makes the pair mean "it went by the safe route" rather than "it did not go".

- [ ] **Step 2: Run to verify they fail**

- [ ] **Step 3: Implement**

```rust
/// How many times to try, and how long to wait between tries.
///
/// Fixed, not exponential: the whole budget is three seconds, so a backoff
/// curve would be arithmetic without a decision behind it. Three seconds is
/// also the ceiling on how far this subscriber lags the hub, which matters
/// because a lagging subscriber is the failure the broadcast channel reports.
const ATTEMPTS: usize = 3;
const RETRY_DELAY: Duration = Duration::from_secs(1);
```

`send` writes the config to stdin via `run_with_stdin` and passes `--config -` plus `--fail`,
`--silent`, `--show-error`, and a `--max-time`. On a non-zero status it sleeps and retries, and
returns the last error after `ATTEMPTS`. **The error text must not contain the URL** — echo curl's
stderr, not the config.

- [ ] **Step 4: Run the suite**

Expected: daemon **74** (71 + 3), core 187, cli 17.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon/src/webhook.rs
git commit -m "feat(daemon): deliver a webhook through curl, with a bounded retry"
```

---

### Task 3: The subscriber task

**Files:**
- Modify: `crates/daemon/src/webhook.rs`, `crates/daemon/src/lib.rs`

**Interfaces:**
- Consumes: `BroadcastSink::subscribe`, `Store::deploy`, `is_notable`, `payload`, `send`
- Produces: `pub fn spawn(state: &AppState, hook: Webhook)` — starts the task

- [ ] **Step 1: Write the failing test**

The task is a loop over a `broadcast::Receiver`; test it by driving the hub directly.

```rust
    /// The event's app name is not on the event — it is on the deploy row —
    /// so the subscriber resolves it. A deploy whose row has vanished must not
    /// take the task down with it.
    #[tokio::test]
    async fn a_notable_event_becomes_one_curl_call_naming_its_app() {
        let (state, _dir) = webhook_harness();          // AppState over fakes
        let id = state.store.create_deploy("web").expect("create");

        spawn(&state, Webhook::new("https://example.com/h".into()));

        let ev = state
            .store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        state.hub.emit(&ev);

        // The task runs concurrently; wait for the call rather than sleeping a
        // fixed amount.
        let calls = await_calls(&exec, 1).await;
        assert_eq!(calls.len(), 1);
    }

    #[tokio::test]
    async fn ordinary_stage_traffic_sends_nothing() {
        // Emit Started and Succeeded; assert no curl call ever appears.
    }
```

`await_calls` polls the `FakeExecutor` with a short timeout and fails if the count is not reached —
**do not use a bare `sleep`**, which makes the test slow when it passes and flaky when it does not.
If the harness makes this awkward, say so and propose the shape rather than settling for a sleep.

- [ ] **Step 2: Run to verify they fail**

- [ ] **Step 3: Implement, and wire it into `serve`**

```rust
/// Subscribe to the hub and send a message for every notable event.
///
/// Its own task, which is the shape `EventSink::emit` was designed for: emit
/// is synchronous and infallible so a deploy cannot be slowed or failed by
/// whoever is listening. All the waiting — the POST, the retry — happens here.
pub fn spawn(state: &AppState, hook: Webhook) { .. }
```

In `serve()`, after the state is built:

```rust
    match webhook::Webhook::from_env() {
        Ok(Some(hook)) => webhook::spawn(&state, hook),
        Ok(None) => {}   // Not configured. Not an error, and not worth a line on every start.
        Err(e) => eprintln!("webhook disabled: {e:#}"),
    }
```

A lagged subscriber re-reads nothing and simply misses those events: a doorbell that missed one ring
is not worth the complexity of the stream's recovery path, and the events table remains the record.
Say that in a comment where the `Lagged` arm is handled.

- [ ] **Step 4: Run the suite**

Expected: daemon **76** (74 + 2), core 187, cli 17.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): send a webhook when a deploy ends or a stage fails"
```

---

### Task 4: `kuadrat serve`, and removing the dead socket field

**Files:**
- Modify: `crates/cli/Cargo.toml`, `crates/cli/src/main.rs`
- Modify: `crates/daemon/src/config.rs`, `crates/daemon/src/lib.rs`
- Delete: `crates/daemon/examples/serve.rs`
- Modify: `docs/design/2026-08-11-phase-3-daemon-and-surfaces.md` (the `--socket` line in the
  configuration example)

**Interfaces:**
- Produces: `kuadrat serve [--listen ADDR] [--root PATH]`

- [ ] **Step 1: Remove `Config.socket`**

The field exists, `serve()` ignores it, and nothing reads it. Remove it from the struct, from
`Default`, and from every construction site, and correct the parent design's configuration example
in the same change so the two do not disagree.

Run the suite: daemon 76, unchanged. A removal that changes a test count removed something else too.

- [ ] **Step 2: Write the failing CLI test**

`crates/cli/src/args.rs` holds the CLI's testable parsing. Add:

```rust
    /// The guard lives in the daemon, but the CLI must not mangle the address
    /// before it gets there.
    #[test]
    fn serve_defaults_to_loopback_on_the_documented_port() {
        assert_eq!(default_listen(), "127.0.0.1:7457".parse::<SocketAddr>().unwrap());
    }
```

Keep this small. The interesting behaviour — refusing a non-loopback address — is already tested in
`crates/daemon/src/config.rs` and must not be duplicated here.

- [ ] **Step 3: Add the subcommand**

`crates/cli/Cargo.toml` gains `kuadrat-daemon = { path = "../daemon" }`.

```rust
    /// Run the HTTP daemon: the API, the pages, and the event stream
    Serve {
        /// Address to listen on. Loopback only — the daemon has no authentication.
        #[arg(long, default_value = "127.0.0.1:7457")]
        listen: SocketAddr,
    },
```

The arm calls `kuadrat_daemon::serve(Config { listen, root })`. `--root` is the existing global flag;
pass it through rather than adding a second one.

- [ ] **Step 4: Delete the example**

`crates/daemon/examples/serve.rs` was a development affordance for opening the pages before this
existed. `kuadrat serve --root /tmp/whatever` now does the same thing, so delete it.

- [ ] **Step 5: Run the suite and try it**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test --workspace 2>&1 | grep "test result"
PATH=$HOME/.cargo/bin:$PATH cargo run -q -p kuadrat -- --root /tmp/kuadrat-h7 serve &
sleep 2 && curl -s -o /dev/null -w "GET / -> %{http_code}\n" http://127.0.0.1:7457/
curl -s -o /dev/null -w "non-loopback refused: " ; PATH=$HOME/.cargo/bin:$PATH cargo run -q -p kuadrat -- serve --listen 0.0.0.0:7457 2>&1 | head -2
kill %1; rm -rf /tmp/kuadrat-h7
```

Expected: cli **18** (17 + 1), core 187, daemon 76; the page answers 200; the non-loopback attempt
prints the refusal naming the SSH tunnel.

- [ ] **Step 6: Commit**

```bash
git add crates docs
git commit -m "feat(cli): kuadrat serve, and drop the dead socket config"
```

---

### Task 5: `kuadrat deploy` against the daemon

**Files:**
- Create: `crates/cli/src/daemon_client.rs`
- Modify: `crates/cli/src/main.rs`

**Interfaces:**
- Produces:
  - `pub enum Handoff { Accepted { deploy_id: i64 }, Unreachable, Refused { status: u16, message: String } }`
  - `pub async fn try_deploy(exec: &dyn Executor, listen: SocketAddr, app: &str) -> Handoff`

**This is the task with the risk in it.** A fallback that cannot tell "the daemon is not there" from
"the daemon said no" becomes a way to bypass the per-app lock: a 409 means a deploy of that app is
already running, and starting a second one locally is the one thing that must not happen.

- [ ] **Step 1: Write the failing tests**

```rust
    #[tokio::test]
    async fn a_refused_connection_means_run_it_here() {
        let exec = FakeExecutor::new();
        // curl exit 7 is "failed to connect to host".
        exec.expect("curl", out(7, "", "Failed to connect to 127.0.0.1 port 7457"));
        assert!(matches!(try_deploy(&exec, addr(), "web").await, Handoff::Unreachable));
    }

    /// The rule this whole module exists for. A 409 says a deploy of this app
    /// is already running; falling back would start a second one and defeat
    /// the lock that makes that impossible everywhere else.
    #[tokio::test]
    async fn a_409_is_the_daemons_answer_and_is_not_retried_locally() {
        let exec = FakeExecutor::new();
        exec.expect("curl", out(22, r#"{"error":"another deploy of web is already in progress"}"#, ""));
        match try_deploy(&exec, addr(), "web").await {
            Handoff::Refused { status, message } => {
                assert_eq!(status, 409);
                assert!(message.contains("already in progress"), "{message}");
            }
            other => panic!("must not fall back: {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_accepted_deploy_returns_the_id_the_daemon_assigned() {
        let exec = FakeExecutor::new();
        exec.expect("curl", out(0, r#"{"deploy_id":12}"#, ""));
        assert!(matches!(
            try_deploy(&exec, addr(), "web").await,
            Handoff::Accepted { deploy_id: 12 }
        ));
    }

    /// An app the daemon has never been told about is a 404 — the operator
    /// registered it nowhere, or typed it wrong. Running it locally would
    /// paper over that.
    #[tokio::test]
    async fn a_404_does_not_fall_back_either() {
        let exec = FakeExecutor::new();
        exec.expect("curl", out(22, r#"{"error":"no app web"}"#, ""));
        assert!(matches!(try_deploy(&exec, addr(), "web").await, Handoff::Refused { .. }));
    }
```

Getting the status code out of `curl` needs `--write-out '%{http_code}'` appended to the body, or
`--fail-with-body` plus parsing. Pick one, and make the parsing a separate function with its own
test — it is the part that can silently misread a response.

- [ ] **Step 2: Run to verify they fail**

- [ ] **Step 3: Implement**

```rust
//! The CLI's thin client for a running daemon.
//!
//! `kuadrat deploy` prefers the daemon when one answers: it queues behind the
//! global one-at-a-time semaphore and gets an addressable `/deploy/:id` page.
//! When nothing answers, the deploy runs in-process exactly as it always has,
//! which is what keeps `kuadrat` a standalone tool and keeps the five existing
//! acceptance scripts working with no daemon at all.
//!
//! The distinction that matters is between *unreachable* and *refused*. Only
//! the first falls back. A refusal is the daemon's answer — a 409 means a
//! deploy of that app is already running, and starting a second one locally
//! would defeat the per-app lock that makes concurrent deploys impossible
//! everywhere else in this system.
```

- [ ] **Step 4: Wire it into the `Deploy` arm**

On `Accepted`, print the deploy id and the URL and exit. On `Refused`, print the daemon's message
and exit non-zero. On `Unreachable`, run the existing in-process path — and say which happened, so
an operator is never guessing where their deploy ran.

- [ ] **Step 5: Run the suite**

Expected: cli **23** (18 + 5), core 187, daemon 76.

- [ ] **Step 6: Commit**

```bash
git add crates/cli
git commit -m "feat(cli): deploy through the daemon when one is running"
```

---

### Task 6: The systemd unit

**Files:**
- Create: `packaging/kuadrat.service`

- [ ] **Step 1: Write the unit**

```ini
[Unit]
Description=kuadrat deploy daemon
After=network.target podman.socket

[Service]
Type=simple
ExecStart=/usr/local/bin/kuadrat serve --listen 127.0.0.1:7457
Restart=on-failure
RestartSec=2

# Deliberately minimal. This service writes Quadlet units into
# /etc/containers/systemd, runs podman, and calls systemctl daemon-reload;
# most of what a hardening template switches on breaks at least one of the
# three. These two are compatible with all of them, and the acceptance script
# completes a real deploy with this file as shipped — a hardening directive
# that was never run is an assumption, not a protection.
NoNewPrivileges=yes
ProtectHome=yes

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 2: Verify it parses**

```bash
systemd-analyze verify packaging/kuadrat.service 2>&1 | head
```

`systemd-analyze verify` warns about the missing binary path if kuadrat is not installed at
`/usr/local/bin`; that warning is expected and is not a failure. Any *syntax* complaint is.

- [ ] **Step 3: Commit**

```bash
git add packaging/kuadrat.service
git commit -m "feat(packaging): a systemd unit for kuadrat serve"
```

---

### Task 7: `scripts/serve-acceptance.sh`

**Files:**
- Create: `scripts/serve-acceptance.sh`

Follow the shape of the five existing scripts — read `scripts/deploy-acceptance.sh` first and match
its structure, its counting, and its output format rather than inventing a sixth style.

- [ ] **Step 1: Write the script**

It must assert, each as its own counted check:

1. the daemon refuses `--listen 0.0.0.0:7457`, and the refusal names the tunnel
2. the daemon starts on loopback and `GET /` answers 200
3. `POST /apps` registers an app and redirects
4. a deploy over the socket reaches `Done`, and the event stream carried six stages
5. `/app/:name` and `/deploy/:id` render
6. the deployed app answers on its port
7. `kuadrat deploy` hands off to the daemon while it is running
8. `kuadrat deploy` runs in-process after the daemon is stopped
9. a webhook POST is attempted on the terminal event — against a local listener, never a real chat
   service

For check 9, run a throwaway receiver (`python3 -m http.server` in a temp dir, or `nc -l`) and point
`KUADRAT_WEBHOOK_URL` at it. Assert the request arrived; do not assert on timing.

- [ ] **Step 2: Shellcheck it**

```bash
shellcheck scripts/serve-acceptance.sh
```

If `shellcheck` is not installed, say so in the report rather than skipping silently.

- [ ] **Step 3: Do NOT run it**

It needs root, and running it installs a unit and deploys a workload on this machine. **Leave that
to Rifky.** Report that it is written and unrun, and give the exact command.

- [ ] **Step 4: Commit**

```bash
git add scripts/serve-acceptance.sh
git commit -m "test(acceptance): exercise the daemon, the pages, and the webhook on a real host"
```

---

### Task 8: Record what changed

**Files:**
- Modify: `docs/design/2026-08-11-phase-3-daemon-and-surfaces.md`, `docs/known-gaps.md`, `README.md`

- [ ] **Step 1: Close the parent design's open questions**

Its "Open questions" section asks about socket activation and about `status`/`list` preferring the
daemon. Replace the socket-activation bullet with the decision and its reason:

```markdown
- **Socket activation: decided against in H7.** The reconcile-before-bind worry turned out to be
  unfounded — a connection waits in the accept queue while `reconcile` runs. The reason not to is
  different: with socket activation the listen address lives in the `.socket` unit, so
  `Config::validate`'s loopback refusal stops being what decides where the daemon is reachable, and
  that guard is in code deliberately. Recorded in `known-gaps.md` rather than closed for good.
```

Leave the `status`/`list` bullet as it is — H7 does not answer it, and saying so is the honest
record.

- [ ] **Step 2: Record the socket-activation gap**

```markdown
## From H7 — no socket activation, and what it would take

The daemon binds its own port, so it holds its memory whenever it runs — a few megabytes that a
socket-activated unit would not. Socket activation was rejected in H7 because it moves the listen
address into the `.socket` unit, where `Config::validate`'s loopback refusal no longer governs it.

Revisit only with a replacement for that guard: the daemon would have to check the address of the
socket it inherits and refuse a non-loopback one, which is the same rule enforced one step later.
```

- [ ] **Step 3: Correct the README**

`README.md` still says there is no daemon or web UI. There is both. Update that sentence and add
the two commands an operator now has: `kuadrat serve`, and the unit in `packaging/`.

- [ ] **Step 4: Commit**

```bash
git add docs README.md
git commit -m "docs: close H7's open questions and describe the daemon in the README"
```

---

## H7 completion checklist

- [ ] `cargo test --workspace` passes: core 187, daemon 76, cli 23
- [ ] `make check` clean
- [ ] The webhook URL appears in no argv, no log line, and no error message
- [ ] An absent webhook URL is silent, not a warning
- [ ] A 4xx from the daemon never falls back to a local deploy; a refused connection always does
- [ ] `kuadrat serve` refuses a non-loopback `--listen`, naming the tunnel
- [ ] `Config.socket` is gone, and the parent design's example no longer shows `--socket`
- [ ] `crates/daemon/examples/serve.rs` is deleted
- [ ] `scripts/serve-acceptance.sh` exists, is shellcheck-clean, and is **documented as unrun**

## Not in H7 (later phases)

- **Live log tailing** and the fleet driver — phase 4.
- **Authentication and the CSRF defence** — recorded in `known-gaps.md` with its trigger.
- **`kuadrat status`/`list` preferring the daemon** — left open, as the parent design leaves it.
- **Socket activation** — recorded, with the guard problem that has to be solved first.
