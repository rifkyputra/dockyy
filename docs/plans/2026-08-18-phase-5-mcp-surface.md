# kuadrat Phase 5 · The MCP Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** An MCP client — Claude Code or anything else — spawns `kuadrat mcp`, lists six tools
(`list_apps`, `get_app`, `deploy`, `get_deploy`, `tail_logs`, `reconcile`), and operates a kuadrat
host through the daemon, with no human relaying CLI output.

**Architecture:** A new `crates/mcp` that never links `kuadrat-core`: it speaks JSON-RPC over
stdio to the client and loopback HTTP (via `curl`, through its own `Daemon` seam) to the daemon.
`kuadrat mcp` is a subcommand on the existing binary; it probes the daemon at startup and refuses
to run without one. One daemon-side addition: `POST /api/reconcile`, so the reconcile tool goes
through the same store and semaphore as everything else.

**Tech Stack:** Rust 2021, tokio, serde_json, async-trait, `curl` as a subprocess, axum (daemon
side only). **No new external dependency in any crate.**

**Spec:** [`docs/design/2026-08-13-phase-5-mcp-surface.md`](../design/2026-08-13-phase-5-mcp-surface.md)

## The protocol moved under the design — read this first

The design says "read the current protocol version out of the specification when implementing."
Done 2026-08-18, and the drift is bigger than a version string: the current MCP revision
**2026-07-28** removed the `initialize` handshake. Modern clients carry the protocol version on
**every request** (`_meta["io.modelcontextprotocol/protocolVersion"]`), servers **MUST** implement
a `server/discover` RPC, and an unsupported version is JSON-RPC error **-32022** listing supported
versions. The handshake the design describes belongs to **legacy** revisions (`2025-11-25` and
earlier) — which is what most deployed clients still speak.

The spec's compatibility matrix says a **dual-era server** ("Legacy client / Dual-era server:
Works", "Modern client / Dual-era server: Works") is the only shape that serves both. So this
plan builds one:

- A request carrying modern `_meta` is served statelessly per 2026-07-28 (`server/discover`,
  `tools/list`, `tools/call`, with `resultType: "complete"` in results).
- An `initialize` request selects legacy semantics for the rest of the process
  (initialize → `notifications/initialized` → `tools/list`/`tools/call`, no `resultType`).
- Anything else before either → a JSON-RPC error; the session survives.

## Global Constraints

- **`crates/mcp` never links `kuadrat-core`** — it is a client of the daemon, full stop. `core`
  is untouched by this phase (ADR-0002 stays trivially true).
- **No new external dependency anywhere.** `crates/mcp` uses only workspace deps that already
  exist: `anyhow`, `serde`, `serde_json`, `tokio`, `async-trait`. The HTTP client is `curl` as a
  subprocess, same as `crates/cli/src/daemon_client.rs`.
- **stdout is sacred.** On stdio, the server MUST NOT write anything to stdout that is not a
  single-line JSON-RPC message. Logs go to stderr. `serde_json::to_string` never emits raw
  newlines (it escapes them), which is what keeps one-message-per-line true by construction.
- **The unreachable/refused distinction is load-bearing**, exactly as `daemon_client.rs`
  documents: curl exit 7 = nothing listening; any completed HTTP exchange = the daemon's answer.
  `--noproxy '*'` on every request so a proxy can never impersonate the daemon.
- **No secrets, no remove, no follow_logs** — per the design, with its reasons. Do not add them.
- **`make check` must pass**: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
- **Prefix cargo commands with `PATH=$HOME/.cargo/bin:$PATH`.**
- **Baselines, measured 2026-08-18 at `20e6f65`:** cli **30**, core **202**, daemon **90**.
- Commit after every task with a Conventional Commit subject.

## Fixed quantities, from the design and the spec

- **Modern protocol version: `"2026-07-28"`.** Legacy versions accepted in `initialize`:
  `"2025-11-25"`, `"2025-06-18"`, `"2025-03-26"`; an unknown requested legacy version is answered
  with `"2025-11-25"` (the legacy rule: respond with your latest when the request is unknown).
- **The daemon address** defaults exactly as `kuadrat serve` does
  (`args::default_listen()` = `127.0.0.1:7457`) and takes the same `--listen` override.
- **`tail_logs` defaults to 100 lines**; the daemon clamps via `core`'s `MAX_LINES`. Not
  re-implemented here.
- **`deploy` returns immediately** with the daemon's `{"deploy_id": N}` (confirmed by Rifky
  2026-08-18); the agent polls `get_deploy`. **`reconcile` is a tool** (same confirmation).
- **No timeout of its own** — the daemon's bounds govern.
- **serverInfo:** `{"name": "kuadrat", "version": env!("CARGO_PKG_VERSION")}`.

## Error handling (three kinds, kept distinguishable)

1. **No daemon at startup** — probe fails → exit non-zero, message names `kuadrat serve`.
2. **The daemon said no** (404/409/400) — a **tool execution error**: `isError: true`, content
   text carrying the daemon's own `error` message. A daemon that dies mid-session surfaces the
   same way, with a message naming `kuadrat serve` (unavoidable once the process is running; the
   startup probe is what keeps it rare).
3. **Malformed client traffic** — JSON-RPC errors, session survives: parse error `-32700`,
   unknown method `-32601`, unknown tool / bad arguments `-32602`, unsupported modern version
   `-32022`, request-before-era-established `-32600`.

---

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` (root) | *Modify.* Add `crates/mcp` to members |
| `crates/mcp/Cargo.toml` | *Create.* Workspace deps only |
| `crates/mcp/src/lib.rs` | *Create.* `serve` loop, era state, dispatch, version gate |
| `crates/mcp/src/rpc.rs` | *Create.* Wire types: parse a line, render response/error lines |
| `crates/mcp/src/daemon.rs` | *Create.* `Daemon` seam: `CurlDaemon`, `FakeDaemon`, `probe` |
| `crates/mcp/src/tools.rs` | *Create.* Six tool definitions + dispatch |
| `crates/daemon/src/api.rs` | *Modify.* `POST /api/reconcile` |
| `crates/cli/Cargo.toml`, `crates/cli/src/main.rs` | *Modify.* The `kuadrat mcp` subcommand |
| `README.md`, `docs/known-gaps.md`, `docs/design/2026-08-13-phase-5-mcp-surface.md` | *Modify.* Record what landed + the protocol addendum |

---

### Task 1: Crate skeleton and the JSON-RPC line loop

**Files:**
- Modify: `Cargo.toml` (root — members)
- Create: `crates/mcp/Cargo.toml`, `crates/mcp/src/lib.rs`, `crates/mcp/src/rpc.rs`

**Interfaces:**
- Produces:
  - `rpc::Incoming { id: Option<serde_json::Value>, method: String, params: serde_json::Value }`
  - `rpc::parse_line(line: &str) -> Result<Incoming, String>` (Err = human-readable parse fault)
  - `rpc::response_line(id: &serde_json::Value, result: serde_json::Value) -> String`
  - `rpc::error_line(id: Option<&serde_json::Value>, code: i64, message: &str, data: Option<serde_json::Value>) -> String`
  - `serve(daemon: &dyn Daemon, reader, writer)` — stub in this task: answers every request
    with `-32601` and ignores notifications (Task 2 gives it real dispatch). Generic:
    `reader: impl AsyncBufRead + Unpin, writer: impl AsyncWrite + Unpin`.
  - (a placeholder `daemon::Daemon` trait so `serve`'s signature is final from the start — Task 3
    fills it in; in this task it is an empty trait)

- [ ] **Step 1: Create the crate**

`crates/mcp/Cargo.toml`:

```toml
[package]
name = "kuadrat-mcp"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
anyhow.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
async-trait.workspace = true
```

Root `Cargo.toml`: `members = ["crates/core", "crates/daemon", "crates/cli", "crates/mcp"]`.

- [ ] **Step 2: Write the failing tests**

In `crates/mcp/src/lib.rs`'s test module. The harness drives `serve` over an in-memory duplex
pipe — this helper is used by every later task's tests too:

```rust
    use tokio::io::AsyncWriteExt;

    /// Feed `lines` to the server, close stdin, and collect every line it
    /// wrote. EOF-driven: `serve` returning on a closed reader is itself part
    /// of what every test asserts.
    async fn session(daemon: &dyn crate::daemon::Daemon, lines: &[serde_json::Value]) -> Vec<serde_json::Value> {
        let (client_side, server_side) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_side);
        let (mut client_write, client_read) = {
            let (r, w) = tokio::io::split(client_side);
            (w, r)
        };

        let mut input = String::new();
        for l in lines {
            input.push_str(&serde_json::to_string(l).expect("test line"));
            input.push('\n');
        }
        client_write.write_all(input.as_bytes()).await.expect("write");
        drop(client_write); // EOF: the server must exit its loop

        let served = crate::serve(daemon, tokio::io::BufReader::new(server_read), server_write);
        let collected = async {
            use tokio::io::AsyncBufReadExt;
            let mut out = Vec::new();
            let mut lines = tokio::io::BufReader::new(client_read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                out.push(serde_json::from_str(&line).expect("server wrote a non-JSON line"));
            }
            out
        };
        let (res, out) = tokio::join!(served, collected);
        res.expect("serve");
        out
    }

    struct NoDaemon;
    impl crate::daemon::Daemon for NoDaemon {}

    #[tokio::test]
    async fn a_parse_error_is_minus_32700_and_the_session_survives() {
        // Feed garbage, then a valid request: the second must still be answered.
        let (client_side, server_side) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_side);
        let (client_read, mut client_write) = {
            let (r, w) = tokio::io::split(client_side);
            (r, w)
        };
        client_write.write_all(b"this is not json\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"nope\"}\n").await.expect("write");
        drop(client_write);

        let served = crate::serve(&NoDaemon, tokio::io::BufReader::new(server_read), server_write);
        let collected = async {
            use tokio::io::AsyncBufReadExt;
            let mut out = Vec::new();
            let mut lines = tokio::io::BufReader::new(client_read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                out.push(serde_json::from_str::<serde_json::Value>(&line).expect("json"));
            }
            out
        };
        let (res, out) = tokio::join!(served, collected);
        res.expect("serve");

        assert_eq!(out[0]["error"]["code"], -32700);
        assert_eq!(out[0]["id"], serde_json::Value::Null);
        assert_eq!(out[1]["error"]["code"], -32601, "the session must survive: {out:?}");
        assert_eq!(out[1]["id"], 1);
    }

    #[tokio::test]
    async fn an_unknown_method_is_minus_32601() {
        let out = session(&NoDaemon, &[serde_json::json!({
            "jsonrpc": "2.0", "id": 7, "method": "resources/list"
        })]).await;
        assert_eq!(out[0]["error"]["code"], -32601);
        assert_eq!(out[0]["id"], 7);
    }

    /// JSON-RPC: a notification gets no response, known method or not.
    #[tokio::test]
    async fn a_notification_gets_no_response_at_all() {
        let out = session(&NoDaemon, &[serde_json::json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        })]).await;
        assert!(out.is_empty(), "{out:?}");
    }

    /// One message per line, and a line may not contain a raw newline — the
    /// stdio framing rule. serde escapes `\n` inside strings; this pins it.
    #[test]
    fn a_rendered_line_never_contains_an_embedded_newline() {
        let line = crate::rpc::response_line(
            &serde_json::json!(1),
            serde_json::json!({ "text": "two\nlines\rand a return" }),
        );
        assert!(!line.contains('\n') && !line.contains('\r'), "{line}");
    }
```

- [ ] **Step 3: Run to verify they fail**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test -p kuadrat-mcp 2>&1 | tail -15
```

Expected: compile failure — the crate has no `serve`, `rpc`, `daemon`.

- [ ] **Step 4: Implement**

`crates/mcp/src/rpc.rs`:

```rust
//! JSON-RPC 2.0 wire shapes, one message per line.
//!
//! Hand-rolled per the design: the subset MCP needs is small enough that an
//! SDK's own error handling would outweigh it. Rendering goes through
//! `serde_json::to_string`, which escapes `\n`/`\r` inside strings — that is
//! what makes "one message per line, no embedded newlines" (the stdio
//! transport's framing rule) hold by construction rather than by audit.

use serde_json::{json, Value};

pub struct Incoming {
    /// `None` marks a notification: it must never be answered.
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

pub fn parse_line(line: &str) -> Result<Incoming, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("parse error: {e}"))?;
    let method = v
        .get("method")
        .and_then(Value::as_str)
        .ok_or("missing method")?
        .to_string();
    Ok(Incoming {
        id: v.get("id").filter(|id| !id.is_null()).cloned(),
        method,
        params: v.get("params").cloned().unwrap_or(Value::Null),
    })
}

pub fn response_line(id: &Value, result: Value) -> String {
    serde_json::to_string(&json!({ "jsonrpc": "2.0", "id": id, "result": result }))
        .expect("a Value serializes")
}

pub fn error_line(id: Option<&Value>, code: i64, message: &str, data: Option<Value>) -> String {
    let mut err = json!({ "code": code, "message": message });
    if let Some(data) = data {
        err["data"] = data;
    }
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "error": err,
    }))
    .expect("a Value serializes")
}
```

`crates/mcp/src/lib.rs` — the loop; dispatch is one `match` that this task leaves almost empty:

```rust
//! The MCP surface: a JSON-RPC-over-stdio server that operates a kuadrat
//! host through the daemon. See docs/design/2026-08-13-phase-5-mcp-surface.md
//! and the plan's protocol addendum: this is a dual-era server (modern
//! 2026-07-28 per-request versioning, plus the legacy initialize handshake).

pub mod daemon;
pub mod rpc;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub async fn serve(
    daemon: &dyn daemon::Daemon,
    reader: impl AsyncBufRead + Unpin,
    writer: impl AsyncWrite + Unpin,
) -> anyhow::Result<()> {
    let _ = daemon;
    let mut writer = writer;
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let out = match rpc::parse_line(&line) {
            Err(fault) => Some(rpc::error_line(None, -32700, &fault, None)),
            Ok(req) => handle(&req),
        };
        if let Some(out) = out {
            writer.write_all(out.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }
    Ok(())
}

/// `None` = notification, nothing to write. Task 2 replaces the body with the
/// real era-aware dispatch; the shape (borrow the request, return the line)
/// is final from the start.
fn handle(req: &rpc::Incoming) -> Option<String> {
    let id = req.id.as_ref()?;
    Some(rpc::error_line(
        Some(id),
        -32601,
        &format!("method not found: {}", req.method),
        None,
    ))
}
```

`crates/mcp/src/daemon.rs`, placeholder for Task 3:

```rust
//! The seam to the kuadrat daemon. Task 3 gives it a real surface; the trait
//! exists from Task 1 so `serve`'s signature never changes.

pub trait Daemon: Send + Sync {}
```

- [ ] **Step 5: Run the suite**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test --workspace 2>&1 | grep -E "Running|test result"
```

Expected: mcp **4**, core 202, daemon 90, cli 30. `make check` clean.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/mcp
git commit -m "feat(mcp): a JSON-RPC line loop over an in-memory-testable pipe"
```

---

### Task 2: The era gate — `server/discover`, `initialize`, and the version rules

**Files:**
- Modify: `crates/mcp/src/lib.rs`

**Interfaces:**
- Consumes: `rpc::*` from Task 1
- Produces:
  - `pub const MODERN_VERSION: &str = "2026-07-28";`
  - `pub const LEGACY_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];`
  - `handle` grows era state: `enum Era { Undetermined, Legacy }` held by the serve loop
  - `fn server_info() -> serde_json::Value` — `{"name":"kuadrat","version":env!("CARGO_PKG_VERSION")}`
  - Methods served: `server/discover`, `initialize`, `notifications/initialized`, `ping`
    (answers `{}` in either era; trivial and clients send it)
  - `fn meta_version(params: &serde_json::Value) -> Option<&str>` — reads
    `params._meta["io.modelcontextprotocol/protocolVersion"]`

- [ ] **Step 1: Write the failing tests**

```rust
    /// The modern path is stateless: discover, then a call carrying _meta —
    /// no initialize anywhere.
    #[tokio::test]
    async fn a_modern_client_needs_no_initialize() {
        let out = session(&NoDaemon, &[
            serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "server/discover",
                "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } } }),
        ]).await;
        let r = &out[0]["result"];
        assert_eq!(r["resultType"], "complete");
        assert_eq!(r["supportedVersions"][0], "2026-07-28");
        assert!(r["capabilities"]["tools"].is_object(), "{r}");
        assert_eq!(r["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "kuadrat");
    }

    /// -32022 with the supported list: the client's retry depends on it.
    #[tokio::test]
    async fn an_unsupported_modern_version_is_minus_32022_naming_supported() {
        let out = session(&NoDaemon, &[
            serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list",
                "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "1900-01-01" } } }),
        ]).await;
        assert_eq!(out[0]["error"]["code"], -32022);
        assert_eq!(out[0]["error"]["data"]["supported"][0], "2026-07-28");
        assert_eq!(out[0]["error"]["data"]["requested"], "1900-01-01");
    }

    /// The legacy handshake: a known requested version is echoed back.
    #[tokio::test]
    async fn initialize_echoes_a_known_legacy_version() {
        let out = session(&NoDaemon, &[
            serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18",
                            "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } } }),
        ]).await;
        let r = &out[0]["result"];
        assert_eq!(r["protocolVersion"], "2025-06-18");
        assert!(r["capabilities"]["tools"].is_object());
        assert_eq!(r["serverInfo"]["name"], "kuadrat");
        assert!(r.get("resultType").is_none(), "legacy results carry no resultType");
    }

    /// The legacy rule for an unknown request: answer with our latest legacy.
    #[tokio::test]
    async fn initialize_with_an_unknown_version_answers_2025_11_25() {
        let out = session(&NoDaemon, &[
            serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "1900-01-01",
                            "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } } }),
        ]).await;
        assert_eq!(out[0]["result"]["protocolVersion"], "2025-11-25");
    }

    /// The design's pin: a tools/call arriving before any era is established
    /// is an error, not a panic — and the session survives to serve the
    /// initialize that follows.
    #[tokio::test]
    async fn a_call_before_any_era_is_an_error_and_the_session_survives() {
        let out = session(&NoDaemon, &[
            serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize",
                "params": { "protocolVersion": "2025-11-25",
                            "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } } }),
        ]).await;
        assert_eq!(out[0]["error"]["code"], -32600);
        let msg = out[0]["error"]["message"].as_str().expect("message");
        assert!(msg.contains("initialize") || msg.contains("protocol version"), "{msg}");
        assert!(out[1]["result"]["protocolVersion"].is_string(), "{:?}", out[1]);
    }
```

- [ ] **Step 2: Run to verify they fail**

Expected: the era tests fail with `-32601` where results were expected.

- [ ] **Step 3: Implement**

In `lib.rs`. The serve loop gains `let mut era = Era::Undetermined;` and passes
`&mut era` to `handle`. The dispatch:

```rust
enum Era {
    Undetermined,
    Legacy,
}

fn meta_version(params: &serde_json::Value) -> Option<&str> {
    params
        .get("_meta")?
        .get("io.modelcontextprotocol/protocolVersion")?
        .as_str()
}

fn server_info() -> serde_json::Value {
    serde_json::json!({ "name": "kuadrat", "version": env!("CARGO_PKG_VERSION") })
}

pub const MODERN_VERSION: &str = "2026-07-28";
pub const LEGACY_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
```

`handle(req, era)` decision order, each arm a small function:

1. `initialize` (request): set `*era = Era::Legacy`. Result:
   `{"protocolVersion": <echo if in LEGACY_VERSIONS, else "2025-11-25">, "capabilities": {"tools": {}}, "serverInfo": server_info()}`.
2. Notifications (`id == None`): return `None` always. (`notifications/initialized` included —
   nothing to do; era is already Legacy.)
3. Modern `_meta` version present:
   - not `MODERN_VERSION` → `error_line(id, -32022, "Unsupported protocol version",
     Some(json!({"supported": [MODERN_VERSION], "requested": v})))`.
   - else serve `server/discover` / `ping` / `tools/list` / `tools/call` (tools arrive Task 4;
     until then answer `-32601`) with `resultType: "complete"` in every result.
     `server/discover` result:
     `{"resultType": "complete", "supportedVersions": [MODERN_VERSION], "capabilities": {"tools": {}}, "_meta": {"io.modelcontextprotocol/serverInfo": server_info()}}`.
4. No `_meta` version, `Era::Legacy` → serve `ping` / `tools/list` / `tools/call` legacy-shaped
   (no `resultType`).
5. No `_meta` version, `Era::Undetermined`:
   - `server/discover` → answer it anyway (it is the compatibility probe; a strict reading
     would refuse the request that exists to prevent misreads).
   - anything else → `-32600`, message: `"send initialize first, or declare a protocol version in _meta (io.modelcontextprotocol/protocolVersion)"`.

- [ ] **Step 4: Run the suite**

Expected: mcp **9** (4 + 5), everything else unchanged. `make check` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/mcp
git commit -m "feat(mcp): dual-era protocol gate — server/discover and the legacy initialize"
```

---

### Task 3: The daemon seam — `CurlDaemon`, `FakeDaemon`, and the startup probe

**Files:**
- Modify: `crates/mcp/src/daemon.rs`

**Interfaces:**
- Produces:
  - `pub enum Method { Get, Post }`
  - `pub enum Answer { Ok { body: String }, Refused { status: Option<u16>, message: String }, Unreachable }`
  - `#[async_trait::async_trait] pub trait Daemon: Send + Sync { async fn request(&self, method: Method, path_and_query: &str) -> Answer; }`
  - `pub struct CurlDaemon { pub listen: std::net::SocketAddr }`
  - `pub fn curl_args(method: Method, url: &str) -> Vec<String>` — pure, so the proxy-bypass
    invariant is testable without running curl
  - `pub async fn probe(daemon: &dyn Daemon) -> anyhow::Result<()>`
  - `pub struct FakeDaemon` with `expect(method, path, answer)` and `calls() -> Vec<(Method, String)>`
  - `pub fn path_segment(s: &str) -> String` — RFC 3986 unreserved-marks encoder, same as
    `daemon_client.rs`'s (reimplemented here: this crate does not link the cli)

- [ ] **Step 1: Write the failing tests**

```rust
    /// curl exit 7 is "failed to connect": the one and only Unreachable.
    /// Everything else completed an HTTP exchange and is the daemon's answer.
    /// (Same taxonomy as crates/cli/src/daemon_client.rs, same reason.)
    #[test]
    fn curl_args_carry_the_proxy_bypass_and_the_status_trailer() {
        let args = curl_args(Method::Post, "http://127.0.0.1:7457/api/apps/web/deploy");
        assert!(args.windows(2).any(|w| w[0] == "--noproxy" && w[1] == "*"), "{args:?}");
        assert!(args.contains(&"--fail-with-body".to_string()), "{args:?}");
        assert!(args.windows(2).any(|w| w[0] == "-w" && w[1] == "\n%{http_code}"), "{args:?}");
        assert!(args.windows(2).any(|w| w[0] == "-X" && w[1] == "POST"), "{args:?}");
    }

    #[test]
    fn split_status_reads_the_appended_code_and_leaves_the_body() {
        let (body, status) = split_status("{\"error\":\"no app web\"}\n404");
        assert_eq!(body, "{\"error\":\"no app web\"}");
        assert_eq!(status, Some(404));
    }

    /// The startup rule: no daemon → an error naming `kuadrat serve`, so an
    /// operator reading the exit message knows the one command to run.
    #[tokio::test]
    async fn probe_with_no_daemon_names_kuadrat_serve() {
        let fake = FakeDaemon::new();
        fake.expect(Method::Get, "/api/apps", Answer::Unreachable);
        let err = probe(&fake).await.unwrap_err();
        assert!(err.to_string().contains("kuadrat serve"), "{err}");
    }

    /// A refusal is an answering daemon: the probe succeeds. (A daemon with a
    /// broken store still serialises deploys; refusing to start the MCP
    /// surface over it would help nobody.)
    #[tokio::test]
    async fn probe_accepts_any_daemon_that_answers() {
        let fake = FakeDaemon::new();
        fake.expect(Method::Get, "/api/apps", Answer::Refused { status: Some(500), message: "broken".into() });
        probe(&fake).await.expect("an answering daemon is a daemon");
    }
```

- [ ] **Step 2: Run to verify they fail**

- [ ] **Step 3: Implement**

`CurlDaemon::request` spawns curl via `tokio::process::Command` (this crate does not have
`core`'s `Executor` — deliberately; the seam for tests is `Daemon`, one level up, as the design
says):

```rust
pub fn curl_args(method: Method, url: &str) -> Vec<String> {
    let mut args = vec!["-sS".to_string()];
    if let Method::Post = method {
        args.push("-X".into());
        args.push("POST".into());
    }
    args.extend([
        "-H".into(),
        "Accept: application/json".into(),
        "--fail-with-body".into(),
        "--noproxy".into(),
        "*".into(),
        "-w".into(),
        "\n%{http_code}".into(),
        url.to_string(),
    ]);
    args
}
```

The exchange logic mirrors `daemon_client.rs` exactly: spawn failure or exit 7 → `Unreachable`;
exit 0 → `Ok { body }` (status line stripped); any other exit → `Refused` with the status from
`split_status` and the message from the body's `error` field (fall back to the trimmed body).
`split_status` and the `error`-field parse are lifted verbatim from `daemon_client.rs` (they are
each a few lines; a shared crate for two ten-line functions would be a third thing to keep in
sync — noted in the module doc).

`probe`: `daemon.request(Method::Get, "/api/apps")`; `Unreachable` →
`bail!("no kuadrat daemon is listening — start one with `kuadrat serve`")`; anything else → `Ok(())`.

`FakeDaemon`: a `Mutex<HashMap<(Method, String), VecDeque<Answer>>>` plus a recorded call list —
same style as `core`'s `FakeExecutor`. An unscripted path returns
`Refused { status: None, message: "unscripted: {method:?} {path}" }` so a test failure names the
missing expectation.

Delete the placeholder empty trait from Task 1; `NoDaemon` in the lib tests becomes
`FakeDaemon::new()` (unscripted = every request refused, which those tests never notice — they
never reach a tool).

- [ ] **Step 4: Run the suite**

Expected: mcp **13** (9 + 4). `make check` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/mcp
git commit -m "feat(mcp): the daemon seam — curl client, fake, startup probe"
```

---

### Task 4: The six tools

**Files:**
- Create: `crates/mcp/src/tools.rs`
- Modify: `crates/mcp/src/lib.rs` (wire `tools/list` + `tools/call` into both eras)

**Interfaces:**
- Consumes: `daemon::{Daemon, Method, Answer, path_segment}`
- Produces:
  - `pub fn definitions() -> serde_json::Value` — the array for `tools/list`
  - `pub enum Dispatched { Result { text: String, is_error: bool }, UnknownTool, BadArguments(String) }`
  - `pub async fn dispatch(name: &str, arguments: &serde_json::Value, daemon: &dyn Daemon) -> Dispatched`

The tool table (name → daemon request):

| Tool | Arguments (JSON Schema) | Request |
|---|---|---|
| `list_apps` | `{}` (no params: `{"type":"object","additionalProperties":false}`) | `GET /api/apps` |
| `get_app` | `name: string` (required) | `GET /api/apps/{name}` |
| `deploy` | `name: string` (required) | `POST /api/apps/{name}/deploy` |
| `get_deploy` | `deploy_id: integer` (required) | `GET /api/deploys/{deploy_id}` |
| `tail_logs` | `name: string` (required), `n: integer` (optional, default 100) | `GET /api/apps/{name}/logs?n={n}` |
| `reconcile` | `{}` | `POST /api/reconcile` |

Descriptions say what an agent needs to choose well; three carry the load:
- `deploy`: "Returns `{deploy_id}` immediately; the deploy runs in the background. Poll
  `get_deploy` to watch its stages. A 409 means a deploy of this app is already running."
- `get_deploy`: "Stage, status, detail, and the full event list for one deploy. `status`
  `in_progress` means keep polling; `succeeded`, `rolled_back`, and `failed` are terminal."
- `reconcile`: "Roll back deploys left in progress by a crash. Waits for the deploy slot, so it
  cannot interrupt a live deploy. Returns the outcomes; an empty list means nothing was stranded."

- [ ] **Step 1: Write the failing tests**

In `tools.rs`'s test module:

```rust
    /// The defect this test exists to catch: a tool advertised but not
    /// dispatchable. Every advertised name must reach the daemon, not
    /// UnknownTool.
    #[tokio::test]
    async fn every_advertised_tool_dispatches() {
        for def in definitions().as_array().expect("array") {
            let name = def["name"].as_str().expect("name");
            let fake = FakeDaemon::new();
            // Script every path the six tools use; each tool will hit one.
            for (m, p) in [
                (Method::Get, "/api/apps"),
                (Method::Get, "/api/apps/web"),
                (Method::Post, "/api/apps/web/deploy"),
                (Method::Get, "/api/deploys/1"),
                (Method::Get, "/api/apps/web/logs?n=100"),
                (Method::Post, "/api/reconcile"),
            ] {
                fake.expect(m, p, Answer::Ok { body: "{}".into() });
            }
            let args = serde_json::json!({ "name": "web", "deploy_id": 1 });
            match dispatch(name, &args, &fake).await {
                Dispatched::Result { .. } => {}
                other => panic!("{name} did not dispatch: {other:?}"),
            }
        }
    }

    /// A 404 is the daemon's answer and must reach the agent as a tool error
    /// carrying that answer — not an empty success (the "quiet app" failure,
    /// one layer out) and not a protocol error (the agent should read it).
    #[tokio::test]
    async fn a_daemon_refusal_is_a_tool_error_carrying_its_message() {
        let fake = FakeDaemon::new();
        fake.expect(Method::Get, "/api/apps/nope", Answer::Refused { status: Some(404), message: "no app nope".into() });
        match dispatch("get_app", &serde_json::json!({ "name": "nope" }), &fake).await {
            Dispatched::Result { text, is_error } => {
                assert!(is_error);
                assert!(text.contains("no app nope"), "{text}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn deploy_passes_through_the_daemons_deploy_id() {
        let fake = FakeDaemon::new();
        fake.expect(Method::Post, "/api/apps/web/deploy", Answer::Ok { body: r#"{"deploy_id":42}"#.into() });
        match dispatch("deploy", &serde_json::json!({ "name": "web" }), &fake).await {
            Dispatched::Result { text, is_error } => {
                assert!(!is_error);
                assert!(text.contains("42"), "{text}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_missing_required_argument_is_bad_arguments_not_a_daemon_call() {
        let fake = FakeDaemon::new();
        match dispatch("get_app", &serde_json::json!({}), &fake).await {
            Dispatched::BadArguments(msg) => assert!(msg.contains("name"), "{msg}"),
            other => panic!("{other:?}"),
        }
        assert!(fake.calls().is_empty(), "the daemon must not have been called");
    }

    #[tokio::test]
    async fn an_unknown_tool_is_unknown_not_a_guess() {
        assert!(matches!(
            dispatch("remove", &serde_json::json!({}), &FakeDaemon::new()).await,
            Dispatched::UnknownTool
        ));
    }

    /// An app name is client input on its way into a URL path.
    #[tokio::test]
    async fn an_app_name_is_percent_encoded_into_the_path() {
        let fake = FakeDaemon::new();
        fake.expect(Method::Get, "/api/apps/a%2Fb", Answer::Ok { body: "{}".into() });
        dispatch("get_app", &serde_json::json!({ "name": "a/b" }), &fake).await;
        assert_eq!(fake.calls()[0].1, "/api/apps/a%2Fb");
    }
```

And in `lib.rs`'s test module, the two ends meet — both eras:

```rust
    #[tokio::test]
    async fn tools_list_matches_the_dispatch_set_in_the_modern_era() {
        let fake = FakeDaemon::new();
        let out = session(&fake, &[serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/list",
            "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } }
        })]).await;
        let tools = out[0]["result"]["tools"].as_array().expect("tools");
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["list_apps", "get_app", "deploy", "get_deploy", "tail_logs", "reconcile"]);
        assert_eq!(out[0]["result"]["resultType"], "complete");
        for t in tools {
            assert!(t["inputSchema"].is_object(), "{t}");
            assert!(t["description"].is_string(), "{t}");
        }
    }

    #[tokio::test]
    async fn a_legacy_session_calls_a_tool_end_to_end() {
        let fake = FakeDaemon::new();
        fake.expect(Method::Get, "/api/apps", Answer::Ok { body: r#"[{"name":"web"}]"#.into() });
        let out = session(&fake, &[
            serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-11-25", "capabilities": {},
                            "clientInfo": { "name": "t", "version": "0" } } }),
            serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "list_apps", "arguments": {} } }),
        ]).await;
        let call = &out[1];
        assert_eq!(call["result"]["isError"], false);
        assert!(call["result"]["content"][0]["text"].as_str().unwrap().contains("web"));
        assert!(call["result"].get("resultType").is_none());
    }

    /// The spec: unknown tool is a PROTOCOL error (-32602), not a tool
    /// error — and the session survives it.
    #[tokio::test]
    async fn an_unknown_tool_name_is_minus_32602_and_the_session_survives() {
        let fake = FakeDaemon::new();
        fake.expect(Method::Get, "/api/apps", Answer::Ok { body: "[]".into() });
        let out = session(&fake, &[
            serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "remove", "arguments": {},
                            "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } } }),
            serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": { "name": "list_apps", "arguments": {},
                            "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } } }),
        ]).await;
        assert_eq!(out[0]["error"]["code"], -32602);
        assert!(out[0]["error"]["message"].as_str().unwrap().contains("remove"));
        assert_eq!(out[1]["result"]["isError"], false, "session must survive: {:?}", out[1]);
    }
```

- [ ] **Step 2: Run to verify they fail**

- [ ] **Step 3: Implement**

`tools.rs`: `definitions()` returns the literal `serde_json::json!` array from the table above.
`dispatch` matches the name, pulls arguments (`BadArguments` when a required one is missing or
mistyped — before any daemon call), builds the path with `path_segment`, and maps the answer:

```rust
    match daemon.request(method, &path).await {
        Answer::Ok { body } => Dispatched::Result { text: body, is_error: false },
        Answer::Refused { message, .. } => Dispatched::Result { text: message, is_error: true },
        Answer::Unreachable => Dispatched::Result {
            text: "the kuadrat daemon stopped answering — restart it with `kuadrat serve` and retry".into(),
            is_error: true,
        },
    }
```

`tail_logs` clamps nothing: it forwards `n` and lets the daemon's `core` clamp, per the design.

In `lib.rs`, both eras route `tools/list` → `definitions()` and `tools/call` →
`dispatch(...)`, differing only in whether `resultType` is added:
- `Dispatched::Result { text, is_error }` →
  `{"content": [{"type": "text", "text": text}], "isError": is_error}` (+ `"resultType": "complete"` when modern).
- `Dispatched::UnknownTool` → `-32602`, message `"Unknown tool: {name}"`.
- `Dispatched::BadArguments(msg)` → `-32602`, message `msg`.

- [ ] **Step 4: Run the suite**

Expected: mcp **22** (13 + 9). `make check` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/mcp
git commit -m "feat(mcp): the six tools, dispatched to the daemon in both eras"
```

---

### Task 5: `POST /api/reconcile` on the daemon

**Files:**
- Modify: `crates/daemon/src/api.rs`

**Interfaces:**
- Consumes: `kuadrat_core::deploy::reconcile`, `AppState::{ctx, deploy_slot}`
- Produces: route `POST /api/reconcile` → `{"reconciled": ["<DeployOutcome debug>", ...]}`

The safety property, stated so the implementer does not lose it: reconcile rolls back anything
`in_progress`. On a **live** daemon an `in_progress` row may be a deploy this very process is
running — so the handler **acquires the deploy slot first**. Holding the only permit means no
deploy is mid-flight in this daemon; whatever is still `in_progress` at that point is genuinely
stranded (a previous process's crash), which is exactly what reconcile exists to roll back. The
CLI's in-process reconcile needs no such guard because no daemon is running when it is correct
to use; this endpoint is how an agent reaches the same recovery *through* the running daemon
without that footgun.

- [ ] **Step 1: Write the failing tests**

In `api.rs`'s test module, using the existing `harness_parts` pattern:

```rust
    #[tokio::test]
    async fn reconcile_with_nothing_stranded_returns_an_empty_list() {
        let (app, _store, _hub, _d) = harness_parts();
        let res = app
            .oneshot(post("/api/reconcile"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(res.into_body(), 1 << 20).await.expect("body"),
        )
        .expect("json");
        assert_eq!(body["reconciled"], serde_json::json!([]));
    }
```

If the module has no `post(path)` helper yet, add one beside `get(path)` with the same shape
(`Request::builder().method("POST")`, `Accept: application/json`, empty body). A second test
covering an actually-stranded row is written only if the harness can seed one without a real
deploy run — check how `core`'s own reconcile tests seed `in_progress` rows
(`crates/core/src/deploy/run.rs`'s test module) and reuse that if the store handle the harness
exposes allows it; if it does not, the empty-list test plus `core`'s existing reconcile coverage
is the honest boundary, and say so in the commit message.

- [ ] **Step 2: Run to verify it fails** (404 — no route)

- [ ] **Step 3: Implement**

```rust
#[derive(Serialize)]
pub struct ReconcileOut {
    pub reconciled: Vec<String>,
}

/// Roll back deploys stranded by a crash — through the daemon, so an agent
/// can reach recovery without a second code path. Waits for the deploy slot:
/// holding the only permit is the proof that no deploy is mid-flight in this
/// process, so nothing live can be mistaken for stranded.
async fn reconcile_api(State(st): State<AppState>) -> ApiResult<Json<ReconcileOut>> {
    let _permit = st
        .deploy_slot
        .acquire()
        .await
        .map_err(|_| ApiError::internal("shutting down".to_string()))?;
    let ctx = st.ctx();
    let outcomes = kuadrat_core::deploy::reconcile(&ctx)
        .await
        .map_err(|e| ApiError::internal(format!("reconcile: {e:#}")))?;
    Ok(Json(ReconcileOut {
        reconciled: outcomes.iter().map(|o| format!("{o:?}")).collect(),
    }))
}
```

Route: `.route("/api/reconcile", post(reconcile_api))` beside the other `/api` routes.

- [ ] **Step 4: Run the suite**

Expected: daemon **91** (90 + 1, or 92 if the stranded-row test proved writable). `make check`
clean.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): reconcile through the daemon, gated by the deploy slot"
```

---

### Task 6: The `kuadrat mcp` subcommand

**Files:**
- Modify: `crates/cli/Cargo.toml`, `crates/cli/src/main.rs`

**Interfaces:**
- Consumes: `kuadrat_mcp::{serve, probe, CurlDaemon}` (via `daemon::CurlDaemon` re-exported —
  add `pub use daemon::{CurlDaemon, probe};` to `crates/mcp/src/lib.rs` if not already public
  at the root)
- Produces: `kuadrat mcp [--listen <addr>]`

- [ ] **Step 1: Wire it**

`crates/cli/Cargo.toml`: `kuadrat-mcp = { path = "../mcp" }`.

In the `Command` enum, beside `Serve`:

```rust
    /// Speak MCP over stdio for an agent, operating the daemon at --listen.
    /// Refuses to start when no daemon answers: an agent cannot see a
    /// fallback message, so a silent second code path would let it report
    /// deploys the daemon's timeline does not contain.
    Mcp {
        /// Daemon address, the same default `kuadrat serve` binds.
        #[arg(long, default_value_t = args::default_listen())]
        listen: std::net::SocketAddr,
    },
```

The match arm:

```rust
        Command::Mcp { listen } => {
            let daemon = kuadrat_mcp::CurlDaemon { listen };
            if let Err(e) = kuadrat_mcp::probe(&daemon).await {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
            kuadrat_mcp::serve(
                &daemon,
                tokio::io::BufReader::new(tokio::io::stdin()),
                tokio::io::stdout(),
            )
            .await?;
        }
```

- [ ] **Step 2: Prove it end to end, no daemon**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo build 2>&1 | tail -3
./target/debug/kuadrat mcp < /dev/null; echo "exit=$?"
```

Expected: a stderr line naming `kuadrat serve`, `exit=1`, nothing on stdout.

- [ ] **Step 3: Prove it end to end, with a daemon**

```bash
ROOT=$(mktemp -d)
./target/debug/kuadrat serve --root "$ROOT" --listen 127.0.0.1:7999 &
SERVE_PID=$!
sleep 1
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_apps","arguments":{},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}' \
  | ./target/debug/kuadrat mcp --listen 127.0.0.1:7999
kill $SERVE_PID
```

Expected: two JSON lines — a `DiscoverResult`, then a `tools/call` result whose content is the
daemon's (empty) app list. If `serve` does not take `--root`, run it bare on the default port
with the same two lines; the probe and the list still exercise the whole path.

- [ ] **Step 4: Run the suite + `make check`**

Expected: counts unchanged from Task 5 (the subcommand is wiring; its behavior is the mcp
crate's, already under test). `make check` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/cli Cargo.lock
git commit -m "feat(cli): kuadrat mcp — the agent surface, daemon-required"
```

---

### Task 7: Record what landed

**Files:**
- Modify: `README.md`, `docs/known-gaps.md`, `docs/design/2026-08-13-phase-5-mcp-surface.md`

- [ ] **Step 1: The design addendum**

Append a dated section to the design doc: the protocol check it mandated found revision
**2026-07-28** (per-request `_meta` versioning, mandatory `server/discover`, `-32022`), the
handshake it described is now the legacy era, and what shipped is dual-era. Two sentences on the
open questions it left: `deploy` returns immediately and `reconcile` is a tool, both confirmed
by Rifky 2026-08-18 before implementation.

- [ ] **Step 2: known-gaps**

Two entries:

```markdown
## From phase 5 — the MCP surface trusts loopback exactly as far as the daemon does

`kuadrat mcp` adds no listener: the client spawns it and owns its pipe, and it talks to the
daemon over the same unauthenticated loopback HTTP every other client uses. It therefore
inherits the auth/CSRF trigger recorded above rather than adding to it — but a host where
untrusted local processes can reach 127.0.0.1:7457 was already trusting them with the daemon,
and the MCP surface makes that reach more convenient. Same trigger, same fix, same change.

## From phase 5 — a daemon that dies mid-session becomes a per-call tool error

The startup probe keeps "no daemon" out of the per-call path, but a daemon that exits after the
probe surfaces as a tool error naming `kuadrat serve` on every subsequent call. An agent may
retry a few times before reading it. Deliberate: exiting the MCP process mid-session would take
the agent's whole surface down to report one dead dependency.
```

- [ ] **Step 3: README**

In the surfaces list, one sentence: the MCP surface (`kuadrat mcp`, stdio) exposes
`list_apps` / `get_app` / `deploy` / `get_deploy` / `tail_logs` / `reconcile` to any MCP
client, requires a running daemon, and deliberately omits `remove` and secrets. Plus the
client-config one-liner:

```bash
claude mcp add kuadrat -- kuadrat mcp
```

- [ ] **Step 4: Run the full gauntlet, then commit**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test --workspace 2>&1 | grep -E "Running|test result"
PATH=$HOME/.cargo/bin:$PATH make check
grep -rn "PreEscaped" crates/ --include="*.rs" | grep -v "//"   # expect nothing
```

```bash
git add README.md docs
git commit -m "docs: record the MCP surface and what it inherits"
```

---

## Completion checklist

- [ ] `cargo test --workspace` passes: core 202 (untouched), daemon 91+, cli 30, mcp 22
- [ ] `make check` clean
- [ ] `crates/mcp` does not link `kuadrat-core` (`grep kuadrat-core crates/mcp/Cargo.toml` → nothing)
- [ ] No new external dependency (`crates/mcp/Cargo.toml` names only workspace deps)
- [ ] A modern client works without `initialize`; a legacy client works with it — both proven by tests
- [ ] Unknown tool → `-32602`; daemon refusal → `isError: true` carrying the daemon's message — proven by tests
- [ ] Startup without a daemon exits non-zero naming `kuadrat serve` — proven end to end
- [ ] `/api/reconcile` acquires the deploy slot before reconciling
- [ ] No secrets, no remove, no follow_logs in the tool list
- [ ] stdout carries only JSON-RPC lines (banner/logs on stderr only)

## Not in this group

- **Authentication and CSRF** — unchanged trigger, recorded in known-gaps.
- **An HTTP MCP transport** — becomes reasonable in the same change that adds authentication.
- **`follow_logs`** — excluded by the design (request/response cannot carry a stream); the
  streaming endpoint keeps serving the page.
- **The fleet driver** — unrelated seam, unchanged.
