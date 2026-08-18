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
