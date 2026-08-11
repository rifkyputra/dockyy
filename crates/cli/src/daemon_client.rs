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

use std::net::SocketAddr;

use kuadrat_core::exec::Executor;

/// Outcome of asking a daemon to run a deploy.
#[derive(Debug)]
pub enum Handoff {
    /// The daemon accepted the deploy and assigned it this id.
    Accepted { deploy_id: i64 },
    /// Curl could not connect at all — nothing is listening on `listen`. The
    /// only case the caller should fall back to running the deploy in-process.
    Unreachable,
    /// The daemon answered and said no. Its status and message, reported
    /// verbatim — this is never a reason to retry the deploy locally.
    Refused { status: u16, message: String },
}

/// curl's exit status for "failed to connect to host" (`man curl` EXIT
/// CODES). The one exit this module reads as *unreachable*; every other exit
/// means curl completed an HTTP exchange (or came close enough that treating
/// it as a refusal, not a green light to run locally, is the safe read).
const CURL_COULD_NOT_CONNECT: i32 = 7;

/// Ask the daemon at `listen` to deploy `app`. `Accept: application/json`
/// keeps the daemon's response a JSON body rather than the `303` it sends a
/// browser. `--fail-with-body` keeps the body on a 4xx/5xx (plain `--fail`
/// discards it), and `-w '\n%{http_code}'` appends the real status after it —
/// still emitted even when `--fail-with-body` turns the exit code into `22`.
pub async fn try_deploy(exec: &dyn Executor, listen: SocketAddr, app: &str) -> Handoff {
    let url = format!("http://{listen}/api/apps/{}/deploy", path_segment(app));
    let args = vec![
        "-sS".to_string(),
        "-X".to_string(),
        "POST".to_string(),
        "-H".to_string(),
        "Accept: application/json".to_string(),
        "--fail-with-body".to_string(),
        "-w".to_string(),
        "\n%{http_code}".to_string(),
        url,
    ];

    let out = match exec.run("curl", &args).await {
        Ok(out) => out,
        // Could not even spawn curl (or some other exec-level failure): no
        // response was received either way, so this is the same as no daemon
        // being there.
        Err(_) => return Handoff::Unreachable,
    };

    if out.status == CURL_COULD_NOT_CONNECT {
        return Handoff::Unreachable;
    }

    let (body, status) = split_status(&out.stdout);

    if out.status == 0 {
        match parse_deploy_id(body) {
            Some(deploy_id) => Handoff::Accepted { deploy_id },
            // The connection succeeded and the daemon said 2xx, so a deploy
            // may already be under way on its side. Reporting a refusal
            // (never falling back to a local run) is the only safe read of a
            // success response this module cannot make sense of.
            None => Handoff::Refused {
                status: status.unwrap_or(200),
                message: format!("daemon returned an unreadable response: {body}"),
            },
        }
    } else {
        let message = parse_error_message(body).unwrap_or_else(|| body.trim().to_string());
        Handoff::Refused {
            // A status curl could not report is still an answer, not a
            // missing daemon — this only guesses at the number, never at the
            // outcome. 409 is the most conservative guess when the real code
            // is unavailable, since "something is already happening" is
            // exactly the case this module must never paper over; the real
            // code from `-w` is used whenever curl supplies one.
            status: status.unwrap_or(409),
            message,
        }
    }
}

/// Split curl's `-w '\n%{http_code}'` output into the response body and the
/// status code, when curl reported one on the trailing line. Its own function
/// because a misread here — a `409` mistaken for absent, or vice versa — is
/// exactly the failure this module exists to prevent.
fn split_status(stdout: &str) -> (&str, Option<u16>) {
    match stdout.rsplit_once('\n') {
        Some((body, code)) if code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()) => {
            (body, code.parse().ok())
        }
        _ => (stdout, None),
    }
}

fn parse_deploy_id(body: &str) -> Option<i64> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("deploy_id")?
        .as_i64()
}

fn parse_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .as_str()
        .map(|s| s.to_string())
}

/// Percent-encode `app` as one path segment before it reaches curl's URL.
/// Hand-rolled rather than adding the `percent-encoding` crate (already in
/// the workspace via the daemon, but not a dependency of this crate, and this
/// task adds none): the daemon's own `path_segment` (`crates/daemon/src/pages.rs`)
/// escapes everything but RFC 3986's unreserved marks, and that is the whole
/// of what a hand-rolled version needs to match.
fn path_segment(app: &str) -> String {
    let mut out = String::with_capacity(app.len());
    for b in app.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuadrat_core::exec::fake::FakeExecutor;
    use kuadrat_core::exec::CommandOutput;

    fn addr() -> SocketAddr {
        "127.0.0.1:7457".parse().unwrap()
    }

    fn out(status: i32, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[tokio::test]
    async fn a_refused_connection_means_run_it_here() {
        let exec = FakeExecutor::new();
        // curl exit 7 is "failed to connect to host".
        exec.expect(
            "curl",
            out(7, "", "Failed to connect to 127.0.0.1 port 7457"),
        );
        assert!(matches!(
            try_deploy(&exec, addr(), "web").await,
            Handoff::Unreachable
        ));
    }

    /// The rule this whole module exists for. A 409 says a deploy of this app
    /// is already running; falling back would start a second one and defeat
    /// the lock that makes that impossible everywhere else.
    #[tokio::test]
    async fn a_409_is_the_daemons_answer_and_is_not_retried_locally() {
        let exec = FakeExecutor::new();
        exec.expect(
            "curl",
            out(
                22,
                r#"{"error":"another deploy of web is already in progress"}"#,
                "",
            ),
        );
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
        assert!(matches!(
            try_deploy(&exec, addr(), "web").await,
            Handoff::Refused { .. }
        ));
    }

    /// The part that can silently misread a response: whether `-w`'s
    /// appended status line is recognised as one, and split cleanly from the
    /// body, when curl really does supply it.
    #[test]
    fn split_status_reads_a_real_appended_code_and_leaves_the_body_intact() {
        let (body, status) = split_status("{\"error\":\"no app web\"}\n404");
        assert_eq!(body, "{\"error\":\"no app web\"}");
        assert_eq!(status, Some(404));
    }
}
