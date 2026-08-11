//! The outbound webhook — the daemon's doorbell.
//!
//! Lives here and not in `core` because `crates/daemon` is, as its own module
//! doc says, the only networked code in kuadrat. It reaches the network the
//! same way everything else reaches the host: through the `Executor` seam,
//! shelling out to `curl`. That buys no new dependency and a sender that is
//! testable with `FakeExecutor` rather than a fake HTTP server.

use anyhow::{Context, Result};
use kuadrat_core::events::{EventKind, EventStatus, StoredEvent};

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

/// Escape a value for a `curl --config` document: `\` first, then `"`, so a
/// literal backslash isn't mistaken for the escape it introduces on the next
/// pass. Also escapes every control character (`< 0x20`, plus `0x7f`) as a
/// `\xHH` sequence — curl's config parser is line-oriented, so a raw
/// character in that range (a newline above all) would corrupt the document
/// structurally, not just the value. This does not depend on what a caller
/// happens to have already escaped: it is the whole guarantee, not half of
/// it shared with `payload`.
fn escape_config_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
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
/// closes that off by escaping every control character, not only `\` and `"`.
pub fn curl_config(url: &str, body: &str) -> String {
    format!(
        "url = \"{}\"\nheader = \"Content-Type: application/json\"\ndata = \"{}\"\n",
        escape_config_value(url),
        escape_config_value(body),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuadrat_core::deploy::{DeployStatus, Stage};
    use kuadrat_core::events::{Event, EventStatus};

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

    #[test]
    fn no_configuration_means_no_webhook() {
        // Both variables absent.
        std::env::remove_var("KUADRAT_WEBHOOK_URL");
        std::env::remove_var("KUADRAT_WEBHOOK_URL_FILE");
        assert!(Webhook::from_env().expect("read").is_none());
    }
}
