//! The seam to the kuadrat daemon: loopback HTTP through `curl`, behind a
//! trait so every test above this line drives a fake instead of a socket.
//!
//! This crate deliberately does not link `kuadrat-core`, so it cannot use
//! `core`'s `Executor` seam — the test seam here is one level up (the whole
//! daemon), which is what the design asks for. The curl mechanics mirror
//! `crates/cli/src/daemon_client.rs` deliberately, including its
//! unreachable/refused taxonomy and its proxy-bypass reasoning: `--noproxy '*'`
//! is what stops a proxy from answering in the daemon's place. `split_status`
//! and the error-field parse are lifted from that module rather than shared
//! with it — a crate for two ten-line functions would be a third thing to keep
//! in sync.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Mutex;

/// The two verbs the six tools need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
}

/// What asking the daemon produced. The unreachable/refused distinction is
/// load-bearing: `Unreachable` means nothing was listening; `Refused` is the
/// daemon's own answer and carries its message.
#[derive(Debug, Clone)]
pub enum Answer {
    Ok {
        body: String,
    },
    Refused {
        status: Option<u16>,
        message: String,
    },
    Unreachable,
}

#[async_trait::async_trait]
pub trait Daemon: Send + Sync {
    async fn request(&self, method: Method, path_and_query: &str) -> Answer;
}

/// curl's exit status for "failed to connect to host" (`man curl` EXIT
/// CODES) — the one exit read as *unreachable*.
const CURL_COULD_NOT_CONNECT: i32 = 7;

/// The daemon over loopback, one `curl` per request.
pub struct CurlDaemon {
    pub listen: SocketAddr,
}

#[async_trait::async_trait]
impl Daemon for CurlDaemon {
    async fn request(&self, method: Method, path_and_query: &str) -> Answer {
        let url = format!("http://{}{}", self.listen, path_and_query);
        let out = match tokio::process::Command::new("curl")
            .args(curl_args(method, &url))
            .output()
            .await
        {
            Ok(out) => out,
            // Could not even spawn curl: no response was received either way.
            Err(_) => return Answer::Unreachable,
        };

        let status = out.status.code().unwrap_or(-1);
        if status == CURL_COULD_NOT_CONNECT {
            return Answer::Unreachable;
        }

        let stdout = String::from_utf8_lossy(&out.stdout);
        let (body, http_status) = split_status(&stdout);
        if status == 0 {
            Answer::Ok {
                body: body.to_string(),
            }
        } else {
            let message = error_message(body).unwrap_or_else(|| body.trim().to_string());
            Answer::Refused {
                status: http_status,
                message,
            }
        }
    }
}

/// The exact flags, pure so the proxy-bypass invariant is testable without
/// running curl. `--fail-with-body` keeps the body on a 4xx/5xx and
/// `-w '\n%{http_code}'` appends the real status after it.
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

/// Split curl's `-w '\n%{http_code}'` output into body and status, when the
/// trailing line really is one.
fn split_status(stdout: &str) -> (&str, Option<u16>) {
    match stdout.rsplit_once('\n') {
        Some((body, code)) if code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()) => {
            (body, code.parse().ok())
        }
        _ => (stdout, None),
    }
}

/// The daemon's `{"error": "..."}` body, when it is one.
fn error_message(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("error")?
        .as_str()
        .map(|s| s.to_string())
}

/// The startup gate: one path, always. A daemon that answers — even with a
/// refusal — is a daemon; only silence is fatal, and the message names the
/// one command that fixes it.
pub async fn probe(daemon: &dyn Daemon) -> anyhow::Result<()> {
    match daemon.request(Method::Get, "/api/apps").await {
        Answer::Unreachable => {
            anyhow::bail!("no kuadrat daemon is listening — start one with `kuadrat serve`")
        }
        _ => Ok(()),
    }
}

/// Percent-encode one path segment: everything but RFC 3986's unreserved
/// marks, matching the daemon's own `path_segment`.
pub fn path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A scripted daemon for tests, in `core`'s `FakeExecutor` style: expectations
/// per (method, path), consumed in order; every call recorded. An unscripted
/// path is a `Refused` naming itself, so a failing test names the missing
/// expectation.
pub struct FakeDaemon {
    scripted: Mutex<HashMap<(Method, String), VecDeque<Answer>>>,
    calls: Mutex<Vec<(Method, String)>>,
}

impl FakeDaemon {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            scripted: Mutex::new(HashMap::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn expect(&self, method: Method, path: &str, answer: Answer) {
        self.scripted
            .lock()
            .expect("lock")
            .entry((method, path.to_string()))
            .or_default()
            .push_back(answer);
    }

    pub fn calls(&self) -> Vec<(Method, String)> {
        self.calls.lock().expect("lock").clone()
    }
}

#[async_trait::async_trait]
impl Daemon for FakeDaemon {
    async fn request(&self, method: Method, path_and_query: &str) -> Answer {
        self.calls
            .lock()
            .expect("lock")
            .push((method, path_and_query.to_string()));
        self.scripted
            .lock()
            .expect("lock")
            .get_mut(&(method, path_and_query.to_string()))
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| Answer::Refused {
                status: None,
                message: format!("unscripted: {method:?} {path_and_query}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// curl exit 7 is "failed to connect": the one and only Unreachable.
    /// Everything else completed an HTTP exchange and is the daemon's answer.
    /// (Same taxonomy as crates/cli/src/daemon_client.rs, same reason.)
    #[test]
    fn curl_args_carry_the_proxy_bypass_and_the_status_trailer() {
        let args = curl_args(Method::Post, "http://127.0.0.1:7457/api/apps/web/deploy");
        assert!(
            args.windows(2).any(|w| w[0] == "--noproxy" && w[1] == "*"),
            "{args:?}"
        );
        assert!(args.contains(&"--fail-with-body".to_string()), "{args:?}");
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-w" && w[1] == "\n%{http_code}"),
            "{args:?}"
        );
        assert!(
            args.windows(2).any(|w| w[0] == "-X" && w[1] == "POST"),
            "{args:?}"
        );
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
        fake.expect(
            Method::Get,
            "/api/apps",
            Answer::Refused {
                status: Some(500),
                message: "broken".into(),
            },
        );
        probe(&fake).await.expect("an answering daemon is a daemon");
    }
}
