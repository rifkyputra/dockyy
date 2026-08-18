//! The six tools, and nothing else. `remove` and the secret commands are
//! deliberately absent — the design's reasons: `remove` is the one
//! irreversible operation, and secrets are stdin-only by construction, a
//! property a JSON tool call cannot provide. `follow_logs` is absent because
//! a tool call is request/response and a stream is not; `tail_logs` is the
//! bounded snapshot an agent can actually read in one turn.

use crate::daemon::{path_segment, Answer, Daemon, Method};
use serde_json::{json, Value};

/// What dispatching a tool produced. `Result` maps to a tool result
/// (`isError` included — the daemon's refusals are tool errors the agent
/// should read); the other two map to JSON-RPC -32602 protocol errors.
#[derive(Debug)]
pub enum Dispatched {
    Result { text: String, is_error: bool },
    UnknownTool,
    BadArguments(String),
}

/// The `tools/list` array. Deterministic order — clients cache the list and
/// stable ordering keeps their prompt caches warm.
pub fn definitions() -> Value {
    json!([
        {
            "name": "list_apps",
            "description": "List every registered app: name, repo path, route, and live host \
                            status (Running, Stopped, Not installed, …).",
            "inputSchema": { "type": "object", "additionalProperties": false }
        },
        {
            "name": "get_app",
            "description": "One registered app by name, with its live host status. An \
                            unregistered name is an error, not an empty result.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string", "description": "The app's registered name" } },
                "required": ["name"],
                "additionalProperties": false
            }
        },
        {
            "name": "deploy",
            "description": "Deploy a registered app. Returns {deploy_id} immediately; the \
                            deploy runs in the background — poll get_deploy to watch its \
                            stages. A 409 means a deploy of this app is already running.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string", "description": "The app's registered name" } },
                "required": ["name"],
                "additionalProperties": false
            }
        },
        {
            "name": "get_deploy",
            "description": "Stage, status, detail, and the full event list for one deploy. \
                            Status in_progress means keep polling; succeeded, rolled_back, \
                            and failed are terminal.",
            "inputSchema": {
                "type": "object",
                "properties": { "deploy_id": { "type": "integer", "description": "The id deploy returned" } },
                "required": ["deploy_id"],
                "additionalProperties": false
            }
        },
        {
            "name": "tail_logs",
            "description": "The last n journal lines for an app (default 100, clamped \
                            server-side). A bounded snapshot — read it in one turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "The app's registered name" },
                    "n": { "type": "integer", "description": "Lines to return (default 100)" }
                },
                "required": ["name"],
                "additionalProperties": false
            }
        },
        {
            "name": "reconcile",
            "description": "Roll back deploys left in progress by a crash. Waits for the \
                            deploy slot, so it cannot interrupt a live deploy. Returns the \
                            outcomes; an empty list means nothing was stranded.",
            "inputSchema": { "type": "object", "additionalProperties": false }
        }
    ])
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or non-string argument: {key}"))
}

fn int_arg(args: &Value, key: &str) -> Result<u64, String> {
    args.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing or non-integer argument: {key}"))
}

/// Route one call to the daemon. Argument faults never reach the daemon.
pub async fn dispatch(name: &str, arguments: &Value, daemon: &dyn Daemon) -> Dispatched {
    let (method, path) = match name {
        "list_apps" => (Method::Get, "/api/apps".to_string()),
        "get_app" => match str_arg(arguments, "name") {
            Ok(n) => (Method::Get, format!("/api/apps/{}", path_segment(n))),
            Err(m) => return Dispatched::BadArguments(m),
        },
        "deploy" => match str_arg(arguments, "name") {
            Ok(n) => (
                Method::Post,
                format!("/api/apps/{}/deploy", path_segment(n)),
            ),
            Err(m) => return Dispatched::BadArguments(m),
        },
        "get_deploy" => match int_arg(arguments, "deploy_id") {
            Ok(id) => (Method::Get, format!("/api/deploys/{id}")),
            Err(m) => return Dispatched::BadArguments(m),
        },
        "tail_logs" => match str_arg(arguments, "name") {
            Ok(n) => {
                let lines = match arguments.get("n") {
                    None => 100,
                    Some(v) => match v.as_u64() {
                        Some(l) => l,
                        None => {
                            return Dispatched::BadArguments("non-integer argument: n".to_string())
                        }
                    },
                };
                (
                    Method::Get,
                    format!("/api/apps/{}/logs?n={lines}", path_segment(n)),
                )
            }
            Err(m) => return Dispatched::BadArguments(m),
        },
        "reconcile" => (Method::Post, "/api/reconcile".to_string()),
        _ => return Dispatched::UnknownTool,
    };

    match daemon.request(method, &path).await {
        Answer::Ok { body } => Dispatched::Result {
            text: body,
            is_error: false,
        },
        Answer::Refused { message, .. } => Dispatched::Result {
            text: message,
            is_error: true,
        },
        Answer::Unreachable => Dispatched::Result {
            text: "the kuadrat daemon stopped answering — restart it with `kuadrat serve` \
                   and retry"
                .to_string(),
            is_error: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{Answer, FakeDaemon, Method};

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
        fake.expect(
            Method::Get,
            "/api/apps/nope",
            Answer::Refused {
                status: Some(404),
                message: "no app nope".into(),
            },
        );
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
        fake.expect(
            Method::Post,
            "/api/apps/web/deploy",
            Answer::Ok {
                body: r#"{"deploy_id":42}"#.into(),
            },
        );
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
        assert!(
            fake.calls().is_empty(),
            "the daemon must not have been called"
        );
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
        fake.expect(
            Method::Get,
            "/api/apps/a%2Fb",
            Answer::Ok { body: "{}".into() },
        );
        dispatch("get_app", &serde_json::json!({ "name": "a/b" }), &fake).await;
        assert_eq!(fake.calls()[0].1, "/api/apps/a%2Fb");
    }
}
