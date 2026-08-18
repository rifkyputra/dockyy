//! The MCP surface: a JSON-RPC-over-stdio server that operates a kuadrat
//! host through the daemon. See `docs/design/2026-08-13-phase-5-mcp-surface.md`
//! and the plan's protocol addendum: this is a dual-era server — modern
//! 2026-07-28 per-request versioning plus the legacy `initialize` handshake.

pub mod daemon;
pub mod rpc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// The current MCP revision: per-request `_meta` versioning, mandatory
/// `server/discover`, no handshake. Read from the specification 2026-08-18,
/// as the design mandates.
pub const MODERN_VERSION: &str = "2026-07-28";

/// Handshake-based revisions this server still answers `initialize` for.
/// An unknown requested version is answered with the first entry — the
/// legacy negotiation rule is "respond with your latest supported".
pub const LEGACY_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

/// Which protocol era this process is speaking. Modern requests are served
/// statelessly; an `initialize` selects legacy semantics for the rest of the
/// process, per the 2026-07-28 spec's dual-era server rules.
enum Era {
    Undetermined,
    Legacy,
}

/// Read newline-delimited JSON-RPC from `reader`, answer on `writer`, return
/// on EOF — which is the stdio transport's graceful-shutdown signal.
pub async fn serve(
    daemon: &dyn daemon::Daemon,
    reader: impl AsyncBufRead + Unpin,
    writer: impl AsyncWrite + Unpin,
) -> anyhow::Result<()> {
    let _ = daemon;
    let mut era = Era::Undetermined;
    let mut writer = writer;
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let out = match rpc::parse_line(&line) {
            Err(fault) => Some(rpc::error_line(None, -32700, &fault, None)),
            Ok(req) => handle(&req, &mut era),
        };
        if let Some(out) = out {
            writer.write_all(out.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }
    Ok(())
}

fn server_info() -> Value {
    json!({ "name": "kuadrat", "version": env!("CARGO_PKG_VERSION") })
}

/// The modern per-request protocol version, when the request declares one.
fn meta_version(params: &Value) -> Option<&str> {
    params
        .get("_meta")?
        .get("io.modelcontextprotocol/protocolVersion")?
        .as_str()
}

/// `None` = notification, nothing to write.
fn handle(req: &rpc::Incoming, era: &mut Era) -> Option<String> {
    // `initialize` selects legacy semantics for the rest of the process,
    // whatever came before it.
    if req.method == "initialize" {
        let id = req.id.as_ref()?;
        *era = Era::Legacy;
        let requested = req
            .params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or("");
        let negotiated = if LEGACY_VERSIONS.contains(&requested) {
            requested
        } else {
            LEGACY_VERSIONS[0]
        };
        return Some(rpc::response_line(
            id,
            json!({
                "protocolVersion": negotiated,
                "capabilities": { "tools": {} },
                "serverInfo": server_info(),
            }),
        ));
    }

    // Notifications are never answered. `notifications/initialized` needs no
    // action either — the era flipped when `initialize` was served.
    let id = req.id.as_ref()?;

    // A method this server never serves, in any era, is -32601 outright.
    // -32600 below is reserved for a method we DO serve arriving before the
    // client has established how it is speaking — a different mistake with a
    // different fix, and the error text says which.
    if !matches!(
        req.method.as_str(),
        "server/discover" | "ping" | "tools/list" | "tools/call"
    ) {
        return Some(rpc::error_line(
            Some(id),
            -32601,
            &format!("method not found: {}", req.method),
            None,
        ));
    }

    if let Some(v) = meta_version(&req.params) {
        // A request carrying modern `_meta` is served statelessly.
        if v != MODERN_VERSION {
            return Some(rpc::error_line(
                Some(id),
                -32022,
                "Unsupported protocol version",
                Some(json!({ "supported": [MODERN_VERSION], "requested": v })),
            ));
        }
        return Some(dispatch(req, id, true));
    }

    match era {
        Era::Legacy => Some(dispatch(req, id, false)),
        // `server/discover` is the compatibility probe; refusing the request
        // that exists to prevent misreads would defeat it. Everything else
        // needs an era first.
        Era::Undetermined if req.method == "server/discover" => Some(dispatch(req, id, true)),
        Era::Undetermined => Some(rpc::error_line(
            Some(id),
            -32600,
            "send initialize first, or declare a protocol version in _meta \
             (io.modelcontextprotocol/protocolVersion)",
            None,
        )),
    }
}

/// One method, one line out. `modern` decides whether results carry the
/// 2026-07-28 `resultType` marker; the method set is the same in both eras.
fn dispatch(req: &rpc::Incoming, id: &Value, modern: bool) -> String {
    match req.method.as_str() {
        "server/discover" => {
            let result = json!({
                "resultType": "complete",
                "supportedVersions": [MODERN_VERSION],
                "capabilities": { "tools": {} },
                "_meta": { "io.modelcontextprotocol/serverInfo": server_info() },
            });
            rpc::response_line(id, result)
        }
        "ping" => {
            let mut result = json!({});
            if modern {
                result["resultType"] = json!("complete");
            }
            rpc::response_line(id, result)
        }
        other => rpc::error_line(
            Some(id),
            -32601,
            &format!("method not found: {other}"),
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;

    /// Feed raw bytes to the server, close its stdin, and collect every line
    /// it wrote. EOF-driven: `serve` returning on a closed reader is itself
    /// part of what every test asserts.
    ///
    /// The client end stays a whole `DuplexStream`: `shutdown()` closes only
    /// its write direction, which is what delivers EOF to the server while
    /// this side keeps reading replies. Splitting it and dropping the write
    /// half would NOT do that — split halves keep the stream alive until
    /// both are gone.
    async fn raw_session(
        daemon: &dyn crate::daemon::Daemon,
        input: &[u8],
    ) -> Vec<serde_json::Value> {
        let (mut client, server_side) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_side);

        client.write_all(input).await.expect("write");
        client.shutdown().await.expect("shutdown"); // EOF for the server

        let served = crate::serve(daemon, tokio::io::BufReader::new(server_read), server_write);
        let collected = async {
            use tokio::io::AsyncBufReadExt;
            let mut out = Vec::new();
            let mut lines = tokio::io::BufReader::new(client).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                out.push(serde_json::from_str(&line).expect("server wrote a non-JSON line"));
            }
            out
        };
        let (res, out) = tokio::join!(served, collected);
        res.expect("serve");
        out
    }

    async fn session(
        daemon: &dyn crate::daemon::Daemon,
        lines: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        let mut input = String::new();
        for l in lines {
            input.push_str(&serde_json::to_string(l).expect("test line"));
            input.push('\n');
        }
        raw_session(daemon, input.as_bytes()).await
    }

    use crate::daemon::FakeDaemon;

    /// An unscripted fake refuses every request — which these protocol-layer
    /// tests never notice, because none of them reach a tool.
    fn no_daemon() -> FakeDaemon {
        FakeDaemon::new()
    }

    #[tokio::test]
    async fn a_parse_error_is_minus_32700_and_the_session_survives() {
        // Feed garbage, then a valid request: the second must still be answered.
        let out = raw_session(
            &no_daemon(),
            b"this is not json\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"nope\"}\n",
        )
        .await;

        assert_eq!(out[0]["error"]["code"], -32700);
        assert_eq!(out[0]["id"], serde_json::Value::Null);
        assert_eq!(
            out[1]["error"]["code"], -32601,
            "the session must survive: {out:?}"
        );
        assert_eq!(out[1]["id"], 1);
    }

    #[tokio::test]
    async fn an_unknown_method_is_minus_32601() {
        let out = session(
            &no_daemon(),
            &[serde_json::json!({
                "jsonrpc": "2.0", "id": 7, "method": "resources/list"
            })],
        )
        .await;
        assert_eq!(out[0]["error"]["code"], -32601);
        assert_eq!(out[0]["id"], 7);
    }

    /// JSON-RPC: a notification gets no response, known method or not.
    #[tokio::test]
    async fn a_notification_gets_no_response_at_all() {
        let out = session(
            &no_daemon(),
            &[serde_json::json!({
                "jsonrpc": "2.0", "method": "notifications/initialized"
            })],
        )
        .await;
        assert!(out.is_empty(), "{out:?}");
    }

    /// The modern path is stateless: discover, then a call carrying _meta —
    /// no initialize anywhere.
    #[tokio::test]
    async fn a_modern_client_needs_no_initialize() {
        let out = session(
            &no_daemon(),
            &[serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "server/discover",
                "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } } })],
        )
        .await;
        let r = &out[0]["result"];
        assert_eq!(r["resultType"], "complete");
        assert_eq!(r["supportedVersions"][0], "2026-07-28");
        assert!(r["capabilities"]["tools"].is_object(), "{r}");
        assert_eq!(
            r["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "kuadrat"
        );
    }

    /// -32022 with the supported list: the client's retry depends on it.
    #[tokio::test]
    async fn an_unsupported_modern_version_is_minus_32022_naming_supported() {
        let out = session(
            &no_daemon(),
            &[serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list",
                "params": { "_meta": { "io.modelcontextprotocol/protocolVersion": "1900-01-01" } } })],
        )
        .await;
        assert_eq!(out[0]["error"]["code"], -32022);
        assert_eq!(out[0]["error"]["data"]["supported"][0], "2026-07-28");
        assert_eq!(out[0]["error"]["data"]["requested"], "1900-01-01");
    }

    /// The legacy handshake: a known requested version is echoed back.
    #[tokio::test]
    async fn initialize_echoes_a_known_legacy_version() {
        let out = session(
            &no_daemon(),
            &[
                serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18",
                            "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } } }),
            ],
        )
        .await;
        let r = &out[0]["result"];
        assert_eq!(r["protocolVersion"], "2025-06-18");
        assert!(r["capabilities"]["tools"].is_object());
        assert_eq!(r["serverInfo"]["name"], "kuadrat");
        assert!(
            r.get("resultType").is_none(),
            "legacy results carry no resultType"
        );
    }

    /// The legacy rule for an unknown request: answer with our latest legacy.
    #[tokio::test]
    async fn initialize_with_an_unknown_version_answers_2025_11_25() {
        let out = session(
            &no_daemon(),
            &[
                serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "1900-01-01",
                            "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } } }),
            ],
        )
        .await;
        assert_eq!(out[0]["result"]["protocolVersion"], "2025-11-25");
    }

    /// The design's pin: a tools/call arriving before any era is established
    /// is an error, not a panic — and the session survives to serve the
    /// initialize that follows.
    #[tokio::test]
    async fn a_call_before_any_era_is_an_error_and_the_session_survives() {
        let out = session(
            &no_daemon(),
            &[
                serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
                serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize",
                    "params": { "protocolVersion": "2025-11-25",
                                "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } } }),
            ],
        )
        .await;
        assert_eq!(out[0]["error"]["code"], -32600);
        let msg = out[0]["error"]["message"].as_str().expect("message");
        assert!(
            msg.contains("initialize") || msg.contains("protocol version"),
            "{msg}"
        );
        assert!(
            out[1]["result"]["protocolVersion"].is_string(),
            "{:?}",
            out[1]
        );
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
}
