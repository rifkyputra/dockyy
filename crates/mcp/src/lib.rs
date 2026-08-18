//! The MCP surface: a JSON-RPC-over-stdio server that operates a kuadrat
//! host through the daemon. See `docs/design/2026-08-13-phase-5-mcp-surface.md`
//! and the plan's protocol addendum: this is a dual-era server — modern
//! 2026-07-28 per-request versioning plus the legacy `initialize` handshake.

pub mod daemon;
pub mod rpc;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// Read newline-delimited JSON-RPC from `reader`, answer on `writer`, return
/// on EOF — which is the stdio transport's graceful-shutdown signal.
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

    struct NoDaemon;
    impl crate::daemon::Daemon for NoDaemon {}

    #[tokio::test]
    async fn a_parse_error_is_minus_32700_and_the_session_survives() {
        // Feed garbage, then a valid request: the second must still be answered.
        let out = raw_session(
            &NoDaemon,
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
            &NoDaemon,
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
            &NoDaemon,
            &[serde_json::json!({
                "jsonrpc": "2.0", "method": "notifications/initialized"
            })],
        )
        .await;
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
}
