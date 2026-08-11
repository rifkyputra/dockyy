//! Bounded journald reads scoped to one kuadrat workload.
//!
//! Both functions go through the [`Executor`] seam like every other host
//! interaction, and both bound their output: this module never runs
//! `journalctl -f`. Live tailing needs a streaming seam that does not exist
//! yet, and arrives in phase 4 where the agent surface needs it too.

use anyhow::{bail, Context, Result};

use crate::exec::Executor;
use crate::workloads::paths::unit_name;

/// Most lines any one read will return, whatever the caller asks for.
///
/// A bound rather than a preference: an unbounded read on a chatty unit can
/// return a great deal of text, and every consumer of this module puts it in
/// an HTTP response.
pub const MAX_LINES: usize = 1000;

/// The last `lines` journal entries for a workload, oldest first.
///
/// `lines` is clamped to `1..=MAX_LINES`. Zero is clamped **up**, because
/// `journalctl -n 0` means "no limit" rather than "no lines".
pub async fn tail(exec: &dyn Executor, name: &str, lines: usize) -> Result<Vec<String>> {
    let args = base_args(name, lines);
    run_journalctl(exec, &args, name).await
}

/// Journal entries for a workload whose message matches `pattern`, oldest
/// first.
///
/// `pattern` is journald's `--grep`, which is a PCRE matched against the
/// message field. It reaches journald as a single argv element — `Executor`
/// takes an argv array and never a shell — so it needs no escaping, but it
/// must not be empty: `--grep ''` matches every line, which is a confusing
/// way to spell `tail`.
///
/// `lines` is clamped to `1..=MAX_LINES`, and bounds the number of *matches*
/// returned rather than the number of entries searched.
pub async fn search(
    exec: &dyn Executor,
    name: &str,
    pattern: &str,
    lines: usize,
) -> Result<Vec<String>> {
    if pattern.is_empty() {
        bail!("search pattern must not be empty; use tail to read without filtering");
    }

    let mut args = base_args(name, lines);
    args.push("--grep".to_string());
    args.push(pattern.to_string());
    run_journalctl(exec, &args, name).await
}

/// The argv every read shares. `--output=short-iso` keeps a timestamp on each
/// line; `--no-pager` stops journald from invoking a pager when stdout is not
/// a terminal.
fn base_args(name: &str, lines: usize) -> Vec<String> {
    let lines = lines.clamp(1, MAX_LINES);
    vec![
        "-u".to_string(),
        unit_name(name),
        "-n".to_string(),
        lines.to_string(),
        "--no-pager".to_string(),
        "--output=short-iso".to_string(),
    ]
}

/// Run journalctl and turn its output into lines.
///
/// Deliberately does **not** pass `-q`. Quiet mode suppresses both the
/// `-- No entries --` marker and the privilege hint, which would make "this
/// unit has logged nothing" and "this process may not read the journal"
/// permanently indistinguishable — and journald exits 0 in both cases.
async fn run_journalctl(exec: &dyn Executor, args: &[String], name: &str) -> Result<Vec<String>> {
    let out = exec
        .run("journalctl", args)
        .await
        .with_context(|| format!("reading logs for {name}"))?;

    if !out.success() {
        bail!("journalctl failed for {name}: {}", out.stderr.trim());
    }

    if journal_unreadable(&out.stderr) {
        bail!(
            "cannot read the system journal for {name}: run as root, or add the \
             user to the systemd-journal group"
        );
    }

    Ok(parse_lines(&out.stdout))
}

/// Detect journald's "you are only seeing your own messages" hint on stderr.
///
/// Matched on the distinctive phrase rather than the whole sentence so a
/// reworded hint in a future systemd still trips it.
fn journal_unreadable(stderr: &str) -> bool {
    stderr.contains("not seeing messages")
}

/// Split journald's stdout into lines, dropping the empty-result marker.
fn parse_lines(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|line| line.trim() != "-- No entries --")
        .map(|line| line.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;

    fn out(status: i32, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[tokio::test]
    async fn tail_asks_journalctl_for_the_prefixed_unit() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "", ""));

        tail(&exec, "My App", 50).await.expect("tail");

        let calls = exec.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "journalctl");
        assert_eq!(
            calls[0].1,
            vec![
                "-u".to_string(),
                "kuadrat-my-app".to_string(),
                "-n".to_string(),
                "50".to_string(),
                "--no-pager".to_string(),
                "--output=short-iso".to_string(),
            ],
            "argv was: {:?}",
            calls[0].1
        );
    }

    #[tokio::test]
    async fn tail_returns_one_string_per_line() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "first\nsecond\nthird\n", ""));

        let lines = tail(&exec, "web", 10).await.expect("tail");
        assert_eq!(lines, vec!["first", "second", "third"]);
    }

    /// journald prints this marker on stdout with exit 0 when a unit has no
    /// entries. Returning it verbatim would render "-- No entries --" in the
    /// UI as though the app had logged it.
    #[tokio::test]
    async fn the_no_entries_marker_becomes_an_empty_vec() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "-- No entries --\n", ""));

        let lines = tail(&exec, "web", 10).await.expect("tail");
        assert!(lines.is_empty(), "got: {lines:?}");
    }

    /// The trap this module exists to avoid: journald exits 0 and prints
    /// "-- No entries --" when the caller cannot read the system journal, so
    /// an unprivileged read is indistinguishable from a quiet app unless the
    /// stderr hint is inspected.
    #[tokio::test]
    async fn an_unreadable_journal_is_an_error_not_an_empty_result() {
        let exec = FakeExecutor::new();
        exec.expect(
            "journalctl",
            out(
                0,
                "-- No entries --\n",
                "Hint: You are currently not seeing messages from other users and the system.\n\
                 Users in groups 'adm', 'systemd-journal' can see all messages.\n",
            ),
        );

        let err = tail(&exec, "web", 10).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("journal"),
            "message should name the journal: {msg}"
        );
        assert!(
            msg.contains("root") || msg.contains("systemd-journal"),
            "message should say how to fix it: {msg}"
        );
    }

    #[tokio::test]
    async fn a_failed_journalctl_is_an_error_carrying_its_stderr() {
        let exec = FakeExecutor::new();
        exec.expect(
            "journalctl",
            out(1, "", "Failed to add match: Invalid argument"),
        );

        let err = tail(&exec, "web", 10).await.unwrap_err();
        assert!(
            err.to_string().contains("Invalid argument"),
            "message was: {err}"
        );
    }

    #[tokio::test]
    async fn a_line_count_above_the_cap_is_clamped() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "", ""));

        tail(&exec, "web", 999_999).await.expect("tail");

        let calls = exec.calls();
        assert!(
            calls[0].1.contains(&MAX_LINES.to_string()),
            "argv should carry the cap, was: {:?}",
            calls[0].1
        );
    }

    /// Zero lines would make journalctl print everything it has — `-n 0` is
    /// not "none" to journald. Clamp up to 1 instead.
    #[tokio::test]
    async fn a_line_count_of_zero_is_clamped_up_to_one() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "", ""));

        tail(&exec, "web", 0).await.expect("tail");

        let calls = exec.calls();
        assert!(
            calls[0].1.contains(&"1".to_string()),
            "argv should carry 1, was: {:?}",
            calls[0].1
        );
    }

    #[tokio::test]
    async fn search_appends_grep_to_the_base_arguments() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "", ""));

        search(&exec, "web", "timeout", 25).await.expect("search");

        let calls = exec.calls();
        assert_eq!(
            calls[0].1,
            vec![
                "-u".to_string(),
                "kuadrat-web".to_string(),
                "-n".to_string(),
                "25".to_string(),
                "--no-pager".to_string(),
                "--output=short-iso".to_string(),
                "--grep".to_string(),
                "timeout".to_string(),
            ],
            "argv was: {:?}",
            calls[0].1
        );
    }

    /// The pattern is caller-supplied and must reach journald as exactly one
    /// argv element. `Executor::run` takes an argv array and never a shell, so
    /// there is no command injection to worry about — but a pattern that got
    /// split on whitespace would silently search for something else, which is
    /// a correctness bug that looks like "search is flaky".
    #[tokio::test]
    async fn a_pattern_with_spaces_stays_a_single_argument() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "", ""));

        search(&exec, "web", "connection refused", 10)
            .await
            .expect("search");

        let calls = exec.calls();
        let argv = &calls[0].1;
        assert!(
            argv.contains(&"connection refused".to_string()),
            "the pattern must be one element, argv was: {argv:?}"
        );
        assert_eq!(
            argv.len(),
            8,
            "argv should be exactly 8 elements, was: {argv:?}"
        );
    }

    /// A pattern that looks like a flag must be searched for, not interpreted.
    /// journald stops treating arguments as options after the value of an
    /// option, so this is really a test that the pattern follows `--grep`
    /// rather than being placed anywhere else.
    #[tokio::test]
    async fn a_pattern_that_looks_like_a_flag_is_still_the_grep_value() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "", ""));

        search(&exec, "web", "--no-pager", 10)
            .await
            .expect("search");

        let calls = exec.calls();
        let argv = &calls[0].1;
        let grep_at = argv
            .iter()
            .position(|a| a == "--grep")
            .expect("--grep present");
        assert_eq!(
            argv[grep_at + 1],
            "--no-pager",
            "the pattern must directly follow --grep, argv was: {argv:?}"
        );
    }

    #[tokio::test]
    async fn search_returns_matching_lines() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "one timeout\ntwo timeout\n", ""));

        let lines = search(&exec, "web", "timeout", 10).await.expect("search");
        assert_eq!(lines, vec!["one timeout", "two timeout"]);
    }

    #[tokio::test]
    async fn search_rejects_an_empty_pattern() {
        let exec = FakeExecutor::new();

        let err = search(&exec, "web", "", 10).await.unwrap_err();
        assert!(err.to_string().contains("pattern"), "message was: {err}");

        assert!(
            exec.calls().is_empty(),
            "an empty pattern must fail before journalctl runs"
        );
    }

    /// search shares tail's privilege handling, and must not lose it.
    #[tokio::test]
    async fn search_also_fails_on_an_unreadable_journal() {
        let exec = FakeExecutor::new();
        exec.expect(
            "journalctl",
            out(
                0,
                "-- No entries --\n",
                "Hint: You are currently not seeing messages from other users and the system.\n",
            ),
        );

        let err = search(&exec, "web", "timeout", 10).await.unwrap_err();
        assert!(err.to_string().contains("journal"), "message was: {err}");
    }
}
