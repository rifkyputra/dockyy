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
use std::path::Path;

use kuadrat_core::exec::Executor;

/// Outcome of asking a daemon to run a deploy.
#[derive(Debug)]
pub enum Handoff {
    /// The daemon accepted the deploy and assigned it this id.
    Accepted { deploy_id: i64 },
    /// Curl could not connect at all — nothing is listening on `listen`. The
    /// only case the caller should fall back to running the deploy in-process.
    Unreachable,
    /// The daemon answered and said no. `status` is the real HTTP status when
    /// curl reported one and `None` when it could not be read — never a
    /// guess standing in for a real number, since a guess presented as fact
    /// can send the operator chasing the wrong cause. Either way this is
    /// never a reason to retry the deploy locally.
    Refused {
        status: Option<u16>,
        message: String,
    },
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
/// `--noproxy '*'` matters more than it looks: curl does not exempt loopback
/// from `http_proxy`/`ALL_PROXY` on its own, so without it a proxy sitting
/// between this process and `listen` can answer in the daemon's place — a
/// dead proxy reads as [`CURL_COULD_NOT_CONNECT`] (a false local fallback)
/// and a live one that 502s reads as [`Handoff::Refused`] (an "answer" that
/// was never the daemon's). Bypassing the proxy is what keeps this module's
/// unreachable/refused distinction meaning what it says.
pub async fn try_deploy(exec: &dyn Executor, listen: SocketAddr, app: &str) -> Handoff {
    let url = format!("http://{listen}/api/apps/{}/deploy", path_segment(app));
    let args = vec![
        "-sS".to_string(),
        "-X".to_string(),
        "POST".to_string(),
        "-H".to_string(),
        "Accept: application/json".to_string(),
        "--fail-with-body".to_string(),
        "--noproxy".to_string(),
        "*".to_string(),
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
                status,
                message: format!("daemon returned an unreadable response: {body}"),
            },
        }
    } else {
        let message = parse_error_message(body).unwrap_or_else(|| body.trim().to_string());
        // `status` is already `None` when curl's `-w` output could not be
        // read off the wire (split_status found no trailing status line).
        // That is reported as unknown, not guessed at: an invented number
        // presented to the operator as the daemon's real status would send
        // them chasing whatever that number conventionally means instead of
        // the actual failure. The safety property does not need the number —
        // any non-2xx exit that is not "could not connect" is a refusal
        // regardless of which status it carries.
        Handoff::Refused { status, message }
    }
}

/// [`try_deploy`], but suppressed entirely when `root` is set.
///
/// `--root` means "do not touch the real host". A daemon that answers on
/// `listen` is, by definition, the real host — this process has no way to
/// learn *that* daemon's own `--root`, so the only way to keep `--root`'s
/// promise is to never ask it for a handoff at all and run locally instead.
/// Returning `Handoff::Unreachable` without calling `exec` (rather than
/// calling `try_deploy` and discarding an `Accepted`) is also what keeps
/// this provable: with `root` set, no `curl` call is recorded, so a test can
/// assert on `exec.calls()` rather than trust a code path it cannot see run.
///
/// This is also what protects the property the module doc leans on: the
/// local fallback's safety from a concurrent deploy holds only because the
/// daemon and a local run share one SQLite file and its per-app lock, which
/// is true only while the two agree on `root`.
pub async fn try_deploy_unless_rooted(
    exec: &dyn Executor,
    listen: SocketAddr,
    app: &str,
    root: Option<&Path>,
) -> Handoff {
    if root.is_some() {
        return Handoff::Unreachable;
    }
    try_deploy(exec, listen, app).await
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

    /// `--root` suppresses the handoff outright, before curl is ever
    /// invoked — not merely discarded after the fact. A running daemon on
    /// the default root cannot be told this process is `--root`-scoped, so
    /// even one that would otherwise happily accept the deploy must never
    /// be asked: this is the property that keeps `--root`'s "do not touch
    /// the real host" promise, and this is what asserting on `exec.calls()`
    /// proves rather than just `Handoff::Unreachable`'s shape.
    #[tokio::test]
    async fn a_root_flag_skips_the_handoff_without_ever_calling_curl() {
        let exec = FakeExecutor::new();
        // Scripted to accept, so a passing test can only mean the call was
        // skipped, not that it happened to fail some other way.
        exec.expect("curl", out(0, r#"{"deploy_id":1}"#, ""));

        let handoff =
            try_deploy_unless_rooted(&exec, addr(), "web", Some(Path::new("/tmp/root"))).await;

        assert!(matches!(handoff, Handoff::Unreachable));
        assert!(
            exec.calls().is_empty(),
            "curl must not run at all when --root is set: {:?}",
            exec.calls()
        );
    }

    /// With no `--root`, `try_deploy_unless_rooted` behaves exactly like
    /// `try_deploy` — the suppression is additive, not a second code path
    /// that could drift from the one every other test in this module covers.
    #[tokio::test]
    async fn with_no_root_the_handoff_runs_normally() {
        let exec = FakeExecutor::new();
        exec.expect("curl", out(0, r#"{"deploy_id":7}"#, ""));

        assert!(matches!(
            try_deploy_unless_rooted(&exec, addr(), "web", None).await,
            Handoff::Accepted { deploy_id: 7 }
        ));
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
    /// the lock that makes that impossible everywhere else. The stdout here
    /// is what curl actually emits: `--fail-with-body` keeps the body, and
    /// `-w '\n%{http_code}'` still appends the real code on its own trailing
    /// line even though the exit code itself is the generic `22` — this is
    /// the fixture that exercises the real parse, not just the default.
    #[tokio::test]
    async fn a_409_is_the_daemons_answer_and_is_not_retried_locally() {
        let exec = FakeExecutor::new();
        exec.expect(
            "curl",
            out(
                22,
                "{\"error\":\"another deploy of web is already in progress\"}\n409",
                "",
            ),
        );
        match try_deploy(&exec, addr(), "web").await {
            Handoff::Refused { status, message } => {
                assert_eq!(status, Some(409));
                assert!(message.contains("already in progress"), "{message}");
            }
            other => panic!("must not fall back: {other:?}"),
        }
    }

    /// When curl's `-w` output is missing entirely — no trailing status
    /// line, as would happen if that flag were ever dropped or mis-parsed —
    /// the status is reported as unknown, not invented. A refusal is still a
    /// refusal either way; only the number curl couldn't provide is absent.
    #[tokio::test]
    async fn a_refusal_with_no_status_line_reports_the_status_as_unknown() {
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
                assert_eq!(status, None);
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

    /// curl does not exempt loopback from `http_proxy`/`ALL_PROXY` on its
    /// own. Without `--noproxy '*'` a proxy sitting in front of `listen`
    /// could answer in the daemon's place — turning a dead proxy into a
    /// false `Unreachable` and a live one's 502 into a `Refused` that never
    /// came from the daemon. Beside `curl_is_asked_to_treat_an_http_error_as_a_failure`
    /// in `webhook.rs`, the same style of assertion for this module's own
    /// invariant.
    #[tokio::test]
    async fn the_deploy_request_bypasses_any_configured_proxy() {
        let exec = FakeExecutor::new();
        exec.expect("curl", out(0, r#"{"deploy_id":1}"#, ""));
        try_deploy(&exec, addr(), "web").await;
        let (_, args) = &exec.calls()[0];
        assert!(
            args.windows(2).any(|w| w[0] == "--noproxy" && w[1] == "*"),
            "{args:?}"
        );
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
