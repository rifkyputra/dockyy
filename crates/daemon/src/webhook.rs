//! The outbound webhook — the daemon's doorbell.
//!
//! Lives here and not in `core` because `crates/daemon` is, as its own module
//! doc says, the only networked code in kuadrat. It reaches the network the
//! same way everything else reaches the host: through the `Executor` seam,
//! shelling out to `curl`. That buys no new dependency and a sender that is
//! testable with `FakeExecutor` rather than a fake HTTP server.

use std::time::Duration;

use anyhow::{Context, Result};
use kuadrat_core::events::{EventKind, EventStatus, StoredEvent};
use kuadrat_core::exec::Executor;

/// Where to POST. Absent configuration means the sender is off — that is not
/// an error and must not warn on every start.
pub struct Webhook {
    url: String,
}

impl Webhook {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// `KUADRAT_WEBHOOK_URL`, else the contents of the file named by
    /// `KUADRAT_WEBHOOK_URL_FILE`.
    ///
    /// A file is offered because a URL carrying a token is a secret, and a
    /// systemd unit's `Environment=` line is readable by anyone who can run
    /// `systemctl show`. `LoadCredential=` and a file keep it out of that.
    pub fn from_env() -> Result<Option<Self>> {
        if let Ok(url) = std::env::var("KUADRAT_WEBHOOK_URL") {
            return Ok(Some(Self::new(url)));
        }
        if let Ok(path) = std::env::var("KUADRAT_WEBHOOK_URL_FILE") {
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("reading webhook URL from {path}"))?;
            return Ok(Some(Self::new(contents.trim().to_string())));
        }
        Ok(None)
    }
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

/// The JSON body: a human-readable `text` line plus the structured fields a
/// receiving webhook (or a curious `jq`) can pick apart. `stage` and `status`
/// come from [`EventKind::columns`] — the same projection the store writes
/// and the JSON API returns — so this never spells a deploy-level event
/// differently than those two surfaces do.
pub fn payload(app: &str, ev: &StoredEvent) -> String {
    let (stage, status) = ev.event.kind.columns();
    let deploy_id = ev.event.deploy_id;
    let text = match &ev.event.detail {
        Some(detail) => format!("{app} #{deploy_id} {stage} {status}: {detail}"),
        None => format!("{app} #{deploy_id} {stage} {status}"),
    };
    serde_json::json!({
        "app": app,
        "deploy_id": deploy_id,
        "stage": stage,
        "status": status,
        "detail": ev.event.detail,
        "text": text,
    })
    .to_string()
}

/// Escape a value for a `curl --config` document.
///
/// `\` first, then `"`, so a literal backslash isn't mistaken for the escape
/// it introduces on the next pass. Per `curl`'s own manual (the `--config`
/// section): inside double quotes it understands exactly six escapes — `\\`,
/// `\"`, `\t`, `\n`, `\r`, `\v` — and **"a backslash preceding any other
/// letter is ignored."** That rules out inventing a `\xHH` form: `\x0a` would
/// not restore the byte, it would arrive as the two literal characters `x0a`,
/// silently wrong rather than visibly rejected. So the four control
/// characters curl names get curl's own spelling, and every other control
/// character (`< 0x20` minus those four, plus `0x7f`) — which curl has no
/// escape for at all — becomes a literal space. A space cannot end the value
/// early (the one property this function exists to guarantee) and cannot be
/// mistaken for content the way `x0a` could.
fn escape_config_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\x0b' => out.push_str("\\v"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// A `curl --config` document that carries the URL and the JSON body without
/// putting either on the command line — argv is world-readable through `ps`,
/// and a webhook URL carries its token in its path.
///
/// The format is line-oriented: each directive is its own line, and a value
/// runs to the closing quote curl finds on that same line. A raw newline
/// inside a value would therefore not stay inside the value — it would end
/// the `data =` line early and hand curl a fresh line to interpret as its
/// *next* option (an `output = /path` embedded in event detail text is not
/// hypothetical: deploy details carry raw command stderr). `escape_config_value`
/// closes that off using curl's own escape set (`\n`, `\r`, `\t`, `\v`, plus
/// `\\` and `\"`) for the characters it understands, and a literal space for
/// any other control byte, which curl has no escape for.
pub fn curl_config(url: &str, body: &str) -> String {
    format!(
        "url = \"{}\"\nheader = \"Content-Type: application/json\"\ndata = \"{}\"\n",
        escape_config_value(url),
        escape_config_value(body),
    )
}

/// How many times to try, and how long to wait between tries.
///
/// Fixed, not exponential: the whole budget is three seconds, so a backoff
/// curve would be arithmetic without a decision behind it. Three seconds is
/// also the ceiling on how far this subscriber lags the hub, which matters
/// because a lagging subscriber is the failure the broadcast channel reports.
const ATTEMPTS: usize = 3;
const RETRY_DELAY: Duration = Duration::from_secs(1);

/// Deliver `body` to `hook` through `curl`, retrying up to [`ATTEMPTS`] times.
///
/// The URL and body reach `curl` on stdin as a `--config` document (see
/// [`curl_config`]) — never on argv, which is world-readable through `ps`.
/// `--fail` makes an HTTP error status a non-zero exit rather than a quiet
/// success; `--silent` and `--show-error` keep curl's own progress meter out
/// of the way while still reporting a real error; `--max-time` bounds a single
/// attempt so a stalled connection can't itself blow the retry budget.
///
/// On failure this returns the last attempt's error so the caller can log
/// it — but the caller is a detached task with a deploy long finished by the
/// time this settles, so nothing here ever blocks or fails a deploy. The
/// returned error carries curl's stderr, never the config document, so the
/// URL's secret token never appears in a log line either.
pub async fn send(exec: &dyn Executor, hook: &Webhook, body: &str) -> Result<()> {
    let config = curl_config(hook.url(), body);
    let args: Vec<String> = [
        "--config",
        "-",
        "--fail",
        "--silent",
        "--show-error",
        "--max-time",
        "10",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let mut last_err = anyhow::anyhow!("webhook delivery attempted zero times");
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(RETRY_DELAY).await;
        }
        match exec.run_with_stdin("curl", &args, &config).await {
            Ok(out) if out.success() => return Ok(()),
            Ok(out) => last_err = anyhow::anyhow!("{}", out.stderr.trim()),
            Err(err) => last_err = err,
        }
    }
    Err(last_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuadrat_core::deploy::{DeployStatus, Stage};
    use kuadrat_core::events::{Event, EventStatus};
    use kuadrat_core::exec::fake::FakeExecutor;
    use kuadrat_core::exec::CommandOutput;

    fn out(status: i32, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    fn stored(id: i64, kind_of: &str) -> StoredEvent {
        let event = match kind_of {
            "finished" => Event::finished(12, DeployStatus::RolledBack, Some("apply broke".into())),
            "failed" => Event::for_stage(
                12,
                Stage::Apply,
                EventStatus::Failed,
                Some("apply broke".into()),
            ),
            "started" => Event::for_stage(12, Stage::Apply, EventStatus::Started, None),
            _ => Event::for_stage(12, Stage::Apply, EventStatus::Succeeded, None),
        };
        StoredEvent {
            id,
            at: "2026-01-01 00:00:00".into(),
            event,
        }
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
        assert!(
            text.contains("web"),
            "the app must be readable in the line: {text}"
        );
        assert!(
            text.contains("12"),
            "the deploy id must be readable: {text}"
        );
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
        assert!(
            cfg.contains(r#"url = "https://example.com/hook/TOKEN""#),
            "{cfg}"
        );
        assert!(cfg.contains("Content-Type: application/json"), "{cfg}");
        assert!(
            cfg.contains(r#"\"text\":\"hi\""#),
            "the body must be escaped for the config: {cfg}"
        );
    }

    /// A quote or a backslash in the body must not end the config value early
    /// — that would truncate the request or, worse, let a log line inject a
    /// curl option. Uses a fixture with no pre-escaped sequences, so this
    /// only passes if `escape_config_value` actually did the work: a bare `"`
    /// and a bare `\` must come out as `\"` and `\\`.
    #[test]
    fn a_body_containing_quotes_and_backslashes_is_escaped() {
        let cfg = curl_config("https://example.com/h", r#"say "hi" C:\x"#);
        let data_line = cfg
            .lines()
            .find_map(|l| l.strip_prefix("data = "))
            .expect("a data line");
        assert_eq!(data_line, r#""say \"hi\" C:\\x""#);
    }

    /// A raw newline in a config value ends the line, and the next line
    /// becomes a curl *option*. Event details carry `format!("{err:#}")` of a
    /// stage failure, which embeds raw command stderr, so this input is
    /// genuinely untrusted — an application that logs a line looking like a
    /// curl option must not become one.
    #[test]
    fn a_newline_in_a_value_cannot_start_a_new_config_line() {
        let cfg = curl_config("https://example.com/h", "line one\noutput = /etc/passwd");
        assert!(
            !cfg.lines().any(|l| l.trim_start().starts_with("output")),
            "a value's newline started a new option line:\n{cfg}"
        );
    }

    /// curl's config parser understands exactly `\\`, `\"`, `\t`, `\n`, `\r`
    /// and `\v`; a backslash before anything else is ignored, so an invented
    /// escape like `\x0a` would arrive as the literal text `x0a`. These four
    /// must therefore use curl's own spellings, not ours.
    #[test]
    fn the_four_control_characters_curl_understands_use_its_own_escapes() {
        let cfg = curl_config("https://example.com/h", "a\nb\tc\rd\x0be");
        let data = cfg
            .lines()
            .find(|l| l.starts_with("data = "))
            .expect("data line");
        assert!(data.contains(r"\n"), "{data}");
        assert!(data.contains(r"\t"), "{data}");
        assert!(data.contains(r"\r"), "{data}");
        assert!(data.contains(r"\v"), "{data}");
        assert!(
            !data.contains(r"\x"),
            "an escape curl does not understand: {data}"
        );
    }

    /// curl has no escape at all for a control character outside its set of
    /// six, so one that arrives (`\x00`, `\x1f`, `0x7f`) is replaced with a
    /// literal space: it keeps the value on one physical line — the safety
    /// property `curl_config` exists to guarantee — without inventing an
    /// escape curl would silently misread.
    #[test]
    fn other_control_characters_become_a_space() {
        let cfg = curl_config("https://example.com/h", "a\x00b\x1fc\x7fd");
        let data_line = cfg
            .lines()
            .find_map(|l| l.strip_prefix("data = "))
            .expect("a data line");
        assert_eq!(data_line, r#""a b c d""#);
    }

    #[test]
    fn no_configuration_means_no_webhook() {
        // Both variables absent.
        std::env::remove_var("KUADRAT_WEBHOOK_URL");
        std::env::remove_var("KUADRAT_WEBHOOK_URL_FILE");
        assert!(Webhook::from_env().expect("read").is_none());
    }

    #[tokio::test]
    async fn a_successful_post_runs_curl_once_with_the_url_on_stdin() {
        let exec = FakeExecutor::new();
        exec.expect("curl", out(0, "", ""));

        send(
            &exec,
            &Webhook::new("https://example.com/h/TOKEN".into()),
            r#"{"text":"x"}"#,
        )
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
        assert!(
            exec.stdins()[0].contains("TOKEN"),
            "the URL must arrive on stdin instead"
        );
    }

    /// Best-effort with a bounded retry: three attempts, then give up. The
    /// deploy is long finished by then and nothing is waiting on this.
    ///
    /// Runs on a paused clock (`start_paused = true`): tokio auto-advances
    /// virtual time past `RETRY_DELAY`'s sleeps whenever the only pending
    /// thing is a timer, so this test verifies the real three-attempt, one
    /// second apart schedule without costing two seconds of wall time.
    #[tokio::test(start_paused = true)]
    async fn a_failing_post_is_retried_three_times_and_then_dropped() {
        let exec = FakeExecutor::new();
        exec.expect("curl", out(7, "", "could not connect"));

        let result = send(&exec, &Webhook::new("https://example.com/h".into()), "{}").await;

        assert!(
            result.is_err(),
            "the caller is told, even though it will only log it"
        );
        assert_eq!(
            exec.calls().len(),
            3,
            "three attempts, not more and not fewer"
        );
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
}
