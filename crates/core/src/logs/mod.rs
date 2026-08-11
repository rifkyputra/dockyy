//! Bounded journald reads scoped to one kuadrat workload.
//!
//! Both functions go through the [`Executor`] seam like every other host
//! interaction, and both bound their output: this module never runs
//! `journalctl -f`. Live tailing needs a streaming seam that does not exist
//! yet, and arrives in phase 4 where the agent surface needs it too.
//!
//! ## The privilege signal depends on an unsuppressed stderr
//!
//! [`journal_unreadable`] tells "this app is quiet" apart from "this process
//! cannot read the journal" by inspecting journald's own stderr hint. That
//! hint is emitted through systemd's logging, which honours
//! `$SYSTEMD_LOG_LEVEL` from the *inherited* environment — [`LocalExecutor`]
//! runs `Command::output()` without clearing or overriding it. If the
//! process kuadrat runs under (or something upstream of it) has set
//! `SYSTEMD_LOG_LEVEL=warning` or higher, journalctl suppresses the hint
//! exactly as `-q` would: empty stderr, exit 0, and this module returns
//! `Ok(vec![])` for a journal it was never able to read. `Executor` has no
//! environment parameter today, so this cannot be fixed inside this module —
//! the daemon must not set `SYSTEMD_LOG_LEVEL` to `warning` or above, and a
//! more durable fix (pinning `SYSTEMD_LOG_LEVEL=info` and `LC_ALL=C` for the
//! journalctl child specifically) belongs with a future `Executor` env
//! parameter, not a workaround here.
//!
//! [`LocalExecutor`]: crate::exec::local::LocalExecutor

use anyhow::{bail, Context, Result};
use tokio_stream::Stream;

use crate::exec::Executor;
use crate::spec::slug;
use crate::workloads::paths::unit_name;

/// Most journal entries any one read will return, whatever the caller asks
/// for.
///
/// This bounds entry *count*, not response size: journald's `LineMax`
/// defaults to 48 KiB per line, so `MAX_LINES` alone does not bound bytes.
/// See [`MAX_LINE_BYTES`] for the per-line bound that closes that gap.
pub const MAX_LINES: usize = 1000;

/// Most bytes any one line will carry before it is truncated.
///
/// journald's own `LineMax` defaults to 48 KiB, and that is how container
/// stdout is captured — one chatty line can be tens of kilobytes wide, and
/// `MAX_LINES` does nothing to stop it. Without this, `MAX_LINES` lines at
/// up to 48 KiB each is a ~48 MB `Vec<String>` that the daemon then
/// serialises whole into one JSON response. Truncation happens in
/// [`parse_lines`], at a char boundary, and leaves a visible marker.
const MAX_LINE_BYTES: usize = 8 * 1024;

/// The last `lines` journal entries for a workload, oldest first.
///
/// `lines` is clamped to `1..=MAX_LINES`. Zero is clamped **up**, because
/// `journalctl -n 0` means "no limit" rather than "no lines".
pub async fn tail(exec: &dyn Executor, name: &str, lines: usize) -> Result<Vec<String>> {
    reject_empty_slug(name)?;
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
    reject_empty_slug(name)?;
    if pattern.is_empty() {
        bail!("search pattern must not be empty; use tail to read without filtering");
    }

    let mut args = base_args(name, lines);
    args.push("--grep".to_string());
    args.push(pattern.to_string());
    run_journalctl(exec, &args, name).await
}

/// Follow a workload's journal: the last `lines` entries, then everything that
/// arrives after.
///
/// Runs the bounded [`tail`] once first, and fails if it does. That is not
/// redundancy: journald reports "you may not read this journal" on stderr while
/// still exiting 0 and printing `-- No entries --` to stdout, so a stream
/// carrying stdout alone cannot tell that apart from a quiet app. `tail`
/// already makes that distinction and is already tested for it; the pre-flight
/// borrows a correct detection rather than writing a weaker one against a data
/// shape that cannot support it.
pub async fn follow(
    exec: &dyn Executor,
    name: &str,
    lines: usize,
) -> Result<Box<dyn Stream<Item = Result<String>> + Send + Unpin>> {
    tail(exec, name, lines).await?;

    let lines = lines.clamp(1, MAX_LINES);
    let args = vec![
        "-u".to_string(),
        unit_name(name),
        "-f".to_string(),
        "-n".to_string(),
        lines.to_string(),
        "--no-pager".to_string(),
        "--output=short-iso".to_string(),
    ];

    exec.run_streaming("journalctl", &args).await
}

/// Reject a workload name that slugs to the empty string before any command
/// runs.
///
/// `WorkloadSpec::validate` rejects this same input for every other entry
/// point into `core` (a name like `"!!!"` has no letter or digit to slug
/// to). Without this check, `tail`/`search` would ask journalctl for unit
/// `kuadrat-`, get no matches, and read that back as "this app is quiet" —
/// the one door in `core` that accepted what the rest of it refuses.
fn reject_empty_slug(name: &str) -> Result<()> {
    if slug(name).is_empty() {
        bail!(
            "workload name {:?} yields an empty identifier; it needs at least one \
             letter or digit",
            name
        );
    }
    Ok(())
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

    // The privilege hint is only decisive when there is nothing else to show:
    // journald derives it from group membership, independently of whether
    // any entries were found, so "hint on stderr AND real lines on stdout"
    // is reachable. Checking `lines.is_empty()` first keeps "empty versus
    // forbidden" distinct without discarding real output on a false
    // positive.
    let lines = parse_lines(&out.stdout);
    if lines.is_empty() && journal_unreadable(&out.stderr) {
        bail!(
            "cannot read the system journal for {name}: run as root, or add the \
             user to the systemd-journal group"
        );
    }

    Ok(lines)
}

/// Detect journald's "you are only seeing your own messages" hint on stderr.
///
/// This is a version-coupled string match, verified against systemd 255 on
/// the validated host — not a semantic parse of the hint. It survives
/// rewording *around* the matched phrases, but a systemd release that
/// rewords the phrases themselves degrades this silently to `Ok(vec![])`
/// rather than failing loudly, because there is no error to distinguish "no
/// hint was printed" from "the hint changed shape". Three phrases are
/// accepted so a partial rewording of any one of them still trips it.
fn journal_unreadable(stderr: &str) -> bool {
    stderr.contains("not seeing messages")
        || stderr.contains("systemd-journal")
        || stderr.contains("Users in groups")
}

/// Split journald's stdout into lines, dropping the empty-result marker and
/// truncating any line over [`MAX_LINE_BYTES`].
fn parse_lines(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|line| line.trim() != "-- No entries --")
        .map(truncate_line)
        .collect()
}

/// Truncate `line` to at most [`MAX_LINE_BYTES`] bytes, cutting at a char
/// boundary — slicing a `str` at an arbitrary byte offset panics if it lands
/// inside a multi-byte character — and appending a visible marker so a
/// truncated line is never mistaken for the app's own output.
fn truncate_line(line: &str) -> String {
    if line.len() <= MAX_LINE_BYTES {
        return line.to_string();
    }

    let mut end = MAX_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }

    format!("{} …[truncated]", &line[..end])
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

    /// A hint on stderr must not shadow real entries on stdout: journald's
    /// privilege hint fires on group membership alone, independent of
    /// whether anything was found, so it must only be treated as an error
    /// when there is nothing else to return.
    #[tokio::test]
    async fn a_privilege_hint_alongside_real_lines_does_not_error() {
        let exec = FakeExecutor::new();
        exec.expect(
            "journalctl",
            out(
                0,
                "first entry\n",
                "Hint: You are currently not seeing messages from other users and the system.\n",
            ),
        );

        let lines = tail(&exec, "web", 10).await.expect("tail");
        assert_eq!(lines, vec!["first entry"]);
    }

    #[tokio::test]
    async fn a_line_over_the_byte_cap_is_truncated_with_a_marker() {
        let exec = FakeExecutor::new();
        let long_line = "a".repeat(MAX_LINE_BYTES + 500);
        exec.expect("journalctl", out(0, &format!("{long_line}\n"), ""));

        let lines = tail(&exec, "web", 10).await.expect("tail");
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].len() < long_line.len(),
            "line should have been shortened"
        );
        assert!(
            lines[0].ends_with("…[truncated]"),
            "line should carry the truncation marker: {}",
            &lines[0][lines[0].len().saturating_sub(40)..]
        );
    }

    /// Truncating must land on a char boundary. A line made entirely of a
    /// multi-byte character whose byte width does not evenly divide
    /// `MAX_LINE_BYTES` would panic on a naive `&line[..MAX_LINE_BYTES]`.
    #[tokio::test]
    async fn a_multi_byte_line_over_the_cap_is_truncated_without_panicking() {
        let exec = FakeExecutor::new();
        // '日' is 3 bytes wide in UTF-8; MAX_LINE_BYTES (8192) is not a
        // multiple of 3, so a naive byte slice would land mid-character.
        let long_line = "日".repeat(MAX_LINE_BYTES);
        exec.expect("journalctl", out(0, &format!("{long_line}\n"), ""));

        let lines = tail(&exec, "web", 10).await.expect("tail");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with("…[truncated]"), "got: {}", lines[0]);
    }

    #[tokio::test]
    async fn a_short_line_is_untouched() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "just a normal line\n", ""));

        let lines = tail(&exec, "web", 10).await.expect("tail");
        assert_eq!(lines, vec!["just a normal line"]);
    }

    /// `journal_unreadable` accepts three phrasings so a partial rewording
    /// of systemd's hint still trips it.
    #[tokio::test]
    async fn each_accepted_hint_phrasing_is_detected() {
        for phrase in [
            "Hint: You are currently not seeing messages from other users and the system.",
            "Users in groups 'adm', 'systemd-journal' can see all messages.",
            "some future wording mentions systemd-journal directly",
        ] {
            let exec = FakeExecutor::new();
            exec.expect("journalctl", out(0, "-- No entries --\n", phrase));

            let err = tail(&exec, "web", 10).await.unwrap_err();
            assert!(
                err.to_string().contains("journal"),
                "phrase {phrase:?} should have been detected, got: {err}"
            );
        }
    }

    /// `WorkloadSpec::validate` rejects a name that slugs to empty; `logs`
    /// must refuse the same input rather than reading "no matches" back as
    /// "this app is quiet".
    #[tokio::test]
    async fn tail_rejects_a_name_that_slugs_to_empty() {
        let exec = FakeExecutor::new();

        let err = tail(&exec, "!!!", 10).await.unwrap_err();
        assert!(
            err.to_string().contains("empty identifier"),
            "message was: {err}"
        );
        assert!(
            exec.calls().is_empty(),
            "a name that slugs to empty must fail before journalctl runs"
        );
    }

    #[tokio::test]
    async fn search_rejects_a_name_that_slugs_to_empty() {
        let exec = FakeExecutor::new();

        let err = search(&exec, "!!!", "timeout", 10).await.unwrap_err();
        assert!(
            err.to_string().contains("empty identifier"),
            "message was: {err}"
        );
        assert!(
            exec.calls().is_empty(),
            "a name that slugs to empty must fail before journalctl runs"
        );
    }

    #[tokio::test]
    async fn follow_asks_journalctl_to_follow_the_prefixed_unit() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "-- No entries --\n", "")); // the pre-flight
        exec.expect_stream("journalctl", vec!["line one".into()]);

        let _stream = follow(&exec, "web", 100).await.expect("follow");

        let (_, args) = &exec.calls()[1];
        assert!(args.iter().any(|a| a == "-u"), "{args:?}");
        assert!(args.iter().any(|a| a == "kuadrat-web"), "{args:?}");
        assert!(args.iter().any(|a| a == "-f"), "{args:?}");
    }

    /// The pre-flight exists for exactly this: journald reports an unreadable
    /// journal on *stderr* while exiting 0, so a stream of stdout alone cannot
    /// tell it from an app that has logged nothing. `tail` already detects it.
    ///
    /// `Result::unwrap_err` requires the `Ok` type to be `Debug`, which a
    /// boxed `dyn Stream` trait object cannot be (a second, non-auto trait
    /// bound isn't allowed on a trait object) — see the same workaround in
    /// `exec::tests::an_executor_that_has_not_opted_in_bails_on_run_streaming`.
    /// Match instead.
    #[tokio::test]
    async fn an_unreadable_journal_fails_before_any_stream_opens() {
        let exec = FakeExecutor::new();
        exec.expect(
            "journalctl",
            out(
                0,
                "-- No entries --\n",
                "Hint: You are currently not seeing messages from other users and the system.\n",
            ),
        );

        let err = match follow(&exec, "web", 100).await {
            Ok(_) => panic!("expected the pre-flight to fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("journal"), "was: {err}");
        assert_eq!(
            exec.calls().len(),
            1,
            "the stream must not have been opened"
        );
    }

    #[tokio::test]
    async fn follows_backlog_is_clamped_like_every_other_read() {
        let exec = FakeExecutor::new();
        exec.expect("journalctl", out(0, "", ""));
        exec.expect_stream("journalctl", vec![]);

        let _stream = follow(&exec, "web", MAX_LINES + 500).await.expect("follow");

        let (_, args) = &exec.calls()[1];
        let n = args
            .iter()
            .position(|a| a == "-n")
            .map(|i| &args[i + 1])
            .expect("-n");
        assert_eq!(n, &MAX_LINES.to_string());
    }
}
