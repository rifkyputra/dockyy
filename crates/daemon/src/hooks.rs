//! Inbound push-to-deploy webhooks: verification and payload parsing.
//!
//! The forge signs what it sends — GitHub with an HMAC-SHA256 over the raw
//! body (`X-Hub-Signature-256`), GitLab with the shared token verbatim
//! (`X-Gitlab-Token`) — and verifying that is the authentication for this
//! surface: no session, no cookie, no browser, so the CSRF trigger recorded
//! in known-gaps stays untripped. Every comparison routes through HMAC
//! (compare `HMAC(k, a)` with `HMAC(k, b)`), which makes equality
//! constant-time without a dedicated constant-time dependency.

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use hmac::{Hmac, Mac};
use kuadrat_core::deploy::DeployStatus;
use kuadrat_core::events::EventSink;
use kuadrat_core::exec::CommandOutput;
use sha2::Sha256;
use std::time::Duration;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

/// The shared hook secret. Absent configuration means the hook routes do not
/// exist (404) — the same off-means-absent contract as the outbound webhook.
pub struct HookSecret(String);

impl HookSecret {
    /// `KUADRAT_HOOK_SECRET`, else the contents of the file named by
    /// `KUADRAT_HOOK_SECRET_FILE` (trimmed).
    ///
    /// A file is offered for the same reason as the outbound webhook URL: a
    /// systemd `Environment=` line is readable by anyone who can run
    /// `systemctl show`; a file via `LoadCredential=` is not.
    pub fn from_env() -> Result<Option<Self>> {
        if let Ok(secret) = std::env::var("KUADRAT_HOOK_SECRET") {
            return Self::new(secret).map(Some);
        }
        if let Ok(path) = std::env::var("KUADRAT_HOOK_SECRET_FILE") {
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("reading hook secret from {path}"))?;
            return Self::new(contents).map(Some);
        }
        Ok(None)
    }

    fn new(secret: String) -> Result<Self> {
        let secret = secret.trim().to_string();
        if secret.is_empty() {
            anyhow::bail!("hook secret must not be empty");
        }
        Ok(Self(secret))
    }

    #[cfg(test)]
    pub fn for_tests(secret: &str) -> Self {
        Self::new(secret.to_string()).expect("non-empty test secret")
    }

    fn mac(&self, data: &[u8]) -> Vec<u8> {
        let mut mac =
            HmacSha256::new_from_slice(self.0.as_bytes()).expect("HMAC accepts any key length");
        mac.update(data);
        mac.finalize().into_bytes().to_vec()
    }
}

/// GitHub: `X-Hub-Signature-256: sha256=<hex>` must be the HMAC-SHA256 of
/// the raw request body under the shared secret.
pub fn verify_github(secret: &HookSecret, body: &[u8], header: Option<&str>) -> bool {
    let Some(hex) = header.and_then(|h| h.strip_prefix("sha256=")) else {
        return false;
    };
    let Some(claimed) = from_hex(hex) else {
        return false;
    };
    let expected = secret.mac(body);
    // HMAC both sides so equality does not short-circuit on the first
    // differing byte of attacker-controlled input.
    secret.mac(&claimed) == secret.mac(&expected)
}

/// GitLab: `X-Gitlab-Token` must equal the shared secret.
pub fn verify_gitlab(secret: &HookSecret, token: Option<&str>) -> bool {
    let Some(token) = token else {
        return false;
    };
    secret.mac(token.as_bytes()) == secret.mac(secret.0.as_bytes())
}

/// One branch push: which branch, and the commit to deploy.
#[derive(Debug, PartialEq, Eq)]
pub struct Push {
    pub branch: String,
    pub sha: String,
}

/// Read `ref` + `after` out of a push payload — the two fields GitHub and
/// GitLab spell identically. Tag pushes, branch deletions (the zero SHA),
/// and non-JSON are `None`: not an error, just not a deployable push.
pub fn parse_push(body: &[u8]) -> Option<Push> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let branch = v
        .get("ref")?
        .as_str()?
        .strip_prefix("refs/heads/")?
        .to_string();
    let sha = v.get("after")?.as_str()?.to_string();
    if !matches!(sha.len(), 40 | 64)
        || !sha.bytes().all(|b| b.is_ascii_hexdigit())
        || sha.bytes().all(|b| b == b'0')
    {
        return None;
    }
    Some(Push { branch, sha })
}

/// GitHub's signed push endpoint. Authentication happens before registration,
/// parsing, or host work so a bad request cannot discover apps or run git.
pub async fn github(
    State(st): State<AppState>,
    Path(app): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(secret) = &st.hook_secret else {
        return ApiError::not_found("not found").into_response();
    };
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|value| value.to_str().ok());
    if !verify_github(secret, &body, signature) {
        return ApiError::unauthorized("unauthorized").into_response();
    }
    dispatch_push(&st, &app, &body).await.into_response()
}

/// GitLab's shared-token push endpoint. It shares every post-authentication
/// step with GitHub; only the provider's verification rule differs.
pub async fn gitlab(
    State(st): State<AppState>,
    Path(app): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(secret) = &st.hook_secret else {
        return ApiError::not_found("not found").into_response();
    };
    let token = headers
        .get("x-gitlab-token")
        .and_then(|value| value.to_str().ok());
    if !verify_gitlab(secret, token) {
        return ApiError::unauthorized("unauthorized").into_response();
    }
    dispatch_push(&st, &app, &body).await.into_response()
}

async fn dispatch_push(st: &AppState, app: &str, body: &[u8]) -> ApiResult<Response> {
    // This guard is shared with normal deploy triggers and registration
    // updates. It closes the check/reset/reserve race and keeps one config
    // snapshot authoritative for this entire delivery.
    let _trigger = st.trigger_lock.lock().await;
    let config = crate::api::registration(st, app)?;
    let Some(push) = parse_push(body) else {
        return Ok(ignored("not a branch push"));
    };

    let repo = &config.repo_path;

    // Do not inspect or mutate a checkout an existing deploy may still be
    // reading. The daemon guard serializes local triggers; the store check and
    // reserve remain the cross-process backstop for direct CLI activity.
    match crate::api::ensure_not_busy(st, app) {
        Ok(()) => {}
        Err(err) if err.is_conflict() => {
            record_refusal(st, app, "hook ignored: deploy in progress")?;
            return Ok(ignored("deploy in progress"));
        }
        Err(err) => return Err(err),
    }

    // Reserve before any git command. The id makes branch-read, fetch, and
    // reset failures durable in the same timeline as every other autonomous
    // deploy attempt, and the per-app lock prevents a cross-process trigger
    // from changing this checkout.
    let deploy_id = match crate::api::reserve_deploy(st, app) {
        Ok(id) => id,
        Err(err) if err.is_conflict() => {
            record_refusal(st, app, "hook ignored: deploy in progress")?;
            return Ok(ignored("deploy in progress"));
        }
        Err(err) => return Err(err),
    };

    let branch = match run_git(
        st,
        app,
        repo,
        &["symbolic-ref", "--short", "HEAD"],
        "reading the checked-out branch",
    )
    .await
    {
        Ok(output) => output.stdout.trim().to_string(),
        Err(err) => {
            finish_attempt(st, deploy_id, "hook git branch read failed")?;
            return Err(err);
        }
    };
    if branch != push.branch {
        let reason = format!("push to {}, deploying {branch}", push.branch);
        finish_attempt(st, deploy_id, &format!("hook ignored: {reason}"))?;
        return Ok(ignored(reason));
    }

    if let Err(err) = run_git(st, app, repo, &["fetch", "origin"], "fetching origin").await {
        finish_attempt(st, deploy_id, "hook git fetch failed")?;
        return Err(err);
    }
    if let Err(err) = run_git(
        st,
        app,
        repo,
        &["reset", "--hard", &push.sha],
        "resetting the checkout",
    )
    .await
    {
        finish_attempt(st, deploy_id, "hook git reset failed")?;
        return Err(err);
    }

    let (spec, repo) = match crate::api::deploy_spec(&config) {
        Ok(prepared) => prepared,
        Err(err) => {
            finish_attempt(st, deploy_id, "hook deploy refused after checkout update")?;
            return Err(err);
        }
    };
    crate::api::spawn_reserved_deploy(st, spec, repo, deploy_id);

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({ "deploy_id": deploy_id })),
    )
        .into_response())
}

fn record_refusal(st: &AppState, app: &str, detail: &str) -> ApiResult<()> {
    let deploy_id = st
        .store
        .create_deploy(app)
        .map_err(|e| ApiError::internal(format!("recording hook refusal: {e:#}")))?;
    finish_attempt(st, deploy_id, detail)
}

fn finish_attempt(st: &AppState, deploy_id: i64, detail: &str) -> ApiResult<()> {
    let stored = st
        .store
        .finish_deploy_with_event(deploy_id, DeployStatus::Failed, Some(detail))
        .map_err(|e| ApiError::internal(format!("finishing hook attempt: {e:#}")))?;
    st.hub.emit(&stored);
    Ok(())
}

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

async fn run_git(
    st: &AppState,
    app: &str,
    repo: &str,
    args: &[&str],
    action: &str,
) -> ApiResult<CommandOutput> {
    let argv: Vec<String> = ["-C", repo]
        .into_iter()
        .chain(args.iter().copied())
        .map(str::to_string)
        .collect();
    let output = tokio::time::timeout(GIT_TIMEOUT, st.exec.run("git", &argv))
        .await
        .map_err(|_| ApiError::internal(format!("git timed out while {action} for {app}")))?
        .map_err(|_| ApiError::internal(format!("git failed while {action} for {app}")))?;
    if !output.success() {
        // Git stderr can echo a credential-bearing remote URL. The forge's
        // delivery log gets a useful stage without receiving that stderr.
        return Err(ApiError::internal(format!(
            "git failed while {action} for {app}"
        )));
    }
    Ok(output)
}

fn ignored(reason: impl Into<String>) -> Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({ "ignored": reason.into() })),
    )
        .into_response()
}

/// Hex-decode without a new crate: `None` on odd length or a non-hex digit.
fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || s.is_empty() {
        return None;
    }
    let digit = |b: u8| -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    };
    s.as_bytes()
        .chunks(2)
        .map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use kuadrat_core::exec::fake::FakeExecutor;
    use kuadrat_core::exec::{CommandOutput, Executor};
    use kuadrat_core::fs::fake::FakeFileSystem;
    use kuadrat_core::spec::WorkloadSpec;
    use kuadrat_core::store::{AppConfig, Store};
    use kuadrat_core::workloads::paths::Paths;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::api::router;
    use crate::state::AppState;

    fn secret() -> HookSecret {
        HookSecret::for_tests("s3cret")
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    struct HookHarness {
        app: Router,
        exec: Arc<FakeExecutor>,
        store: Arc<Store>,
        repo: String,
        _dir: TempDir,
    }

    fn hook_harness(configured: bool) -> HookHarness {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        std::fs::write(
            repo.join("kuadrat.json"),
            serde_json::to_string(&WorkloadSpec::new("web", "placeholder")).expect("spec"),
        )
        .expect("write spec");
        let repo = repo.to_string_lossy().into_owned();

        let store = Arc::new(Store::open(&dir.path().join("k.db")).expect("store"));
        store
            .register_app(&AppConfig {
                name: "web".into(),
                repo_path: repo.clone(),
                route: None,
            })
            .expect("register");

        let exec = Arc::new(FakeExecutor::new());
        exec.expect_call(
            "git",
            &["-C", &repo, "symbolic-ref", "--short", "HEAD"],
            ok("main\n"),
        );
        exec.expect_call("git", &["-C", &repo, "fetch", "origin"], ok(""));
        exec.expect_call(
            "git",
            &[
                "-C",
                &repo,
                "reset",
                "--hard",
                "1111111111111111111111111111111111111111",
            ],
            ok(""),
        );

        let mut state = AppState::new(
            exec.clone(),
            Arc::new(FakeFileSystem::new()),
            store.clone(),
            Paths::rooted(dir.path()),
        );
        if configured {
            state.hook_secret = Some(Arc::new(secret()));
        }

        HookHarness {
            app: router(state),
            exec,
            store,
            repo,
            _dir: dir,
        }
    }

    fn github_header(body: &[u8]) -> String {
        let digest = secret().mac(body);
        format!(
            "sha256={}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        )
    }

    fn hook_request(path: &str, body: &'static [u8], header: (&str, String)) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path)
            .header(header.0, header.1)
            .body(Body::from(body))
            .expect("request")
    }

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    /// Known-answer, computed independently (python3 hmac): pins our parsing
    /// and hex handling, not just crate round-tripping.
    #[test]
    fn a_correctly_signed_github_body_verifies() {
        let body =
            br#"{"ref":"refs/heads/main","after":"1111111111111111111111111111111111111111"}"#;
        let header = "sha256=a3450400315a03375f96c5ed76f59082b5f4f39ccfd1ad04f1df07ecba18f809";
        assert!(verify_github(&secret(), body, Some(header)));
    }

    #[test]
    fn a_tampered_body_does_not_verify() {
        let body =
            br#"{"ref":"refs/heads/evil","after":"1111111111111111111111111111111111111111"}"#;
        let header = "sha256=a3450400315a03375f96c5ed76f59082b5f4f39ccfd1ad04f1df07ecba18f809";
        assert!(!verify_github(&secret(), body, Some(header)));
    }

    #[test]
    fn a_missing_or_malformed_header_does_not_verify() {
        let body = b"x";
        assert!(!verify_github(&secret(), body, None));
        assert!(!verify_github(&secret(), body, Some("sha256=zz")));
        assert!(!verify_github(&secret(), body, Some("md5=abcd")));
        assert!(!verify_github(&secret(), body, Some("")));
    }

    #[test]
    fn the_gitlab_token_verifies_only_on_exact_match() {
        assert!(verify_gitlab(&secret(), Some("s3cret")));
        assert!(!verify_gitlab(&secret(), Some("s3cret ")));
        assert!(!verify_gitlab(&secret(), Some("wrong")));
        assert!(!verify_gitlab(&secret(), None));
    }

    #[test]
    fn an_empty_or_whitespace_only_secret_is_rejected() {
        assert!(HookSecret::new(String::new()).is_err());
        assert!(HookSecret::new(" \n\t ".into()).is_err());
    }

    #[test]
    fn parse_push_reads_branch_and_sha_and_ignores_deletions() {
        let push = parse_push(
            br#"{"ref":"refs/heads/main","after":"1111111111111111111111111111111111111111"}"#,
        )
        .expect("a branch push parses");
        assert_eq!(push.branch, "main");
        assert_eq!(push.sha, "1111111111111111111111111111111111111111");

        // A tag push is not a branch push.
        assert!(parse_push(br#"{"ref":"refs/tags/v1","after":"1111"}"#).is_none());
        // A branch deletion carries the zero SHA.
        assert!(parse_push(
            br#"{"ref":"refs/heads/main","after":"0000000000000000000000000000000000000000"}"#
        )
        .is_none());
        // A value that could be interpreted as a git option is not a commit.
        assert!(parse_push(br#"{"ref":"refs/heads/main","after":"--help"}"#).is_none());
        // Garbage is not a push.
        assert!(parse_push(b"not json").is_none());
    }

    #[tokio::test]
    async fn a_signed_github_push_updates_the_repo_and_deploys() {
        let h = hook_harness(true);
        let body =
            br#"{"ref":"refs/heads/main","after":"1111111111111111111111111111111111111111"}"#;
        let response = h
            .app
            .oneshot(hook_request(
                "/hooks/github/web",
                body,
                ("x-hub-signature-256", github_header(body)),
            ))
            .await
            .expect("send");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_json(response).await.get("deploy_id").is_some());
        let calls = h.exec.calls();
        assert!(calls.iter().any(|(program, args)| {
            program == "git" && args == &["-C", &h.repo, "fetch", "origin"]
        }));
        assert!(calls.iter().any(|(program, args)| {
            program == "git"
                && args
                    == &[
                        "-C",
                        &h.repo,
                        "reset",
                        "--hard",
                        "1111111111111111111111111111111111111111",
                    ]
        }));
    }

    #[tokio::test]
    async fn a_bad_signature_is_401_and_runs_no_git() {
        let h = hook_harness(true);
        let body =
            br#"{"ref":"refs/heads/main","after":"1111111111111111111111111111111111111111"}"#;
        let response = h
            .app
            .oneshot(hook_request(
                "/hooks/github/web",
                body,
                ("x-hub-signature-256", "sha256=00".into()),
            ))
            .await
            .expect("send");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(h.exec.calls().is_empty(), "no command may run before auth");
    }

    #[tokio::test]
    async fn no_secret_configured_means_404_before_any_work() {
        let h = hook_harness(false);
        let body =
            br#"{"ref":"refs/heads/main","after":"1111111111111111111111111111111111111111"}"#;
        let response = h
            .app
            .oneshot(hook_request(
                "/hooks/github/web",
                body,
                ("x-hub-signature-256", github_header(body)),
            ))
            .await
            .expect("send");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(h.exec.calls().is_empty(), "an absent route does no work");
    }

    #[tokio::test]
    async fn a_push_to_another_branch_is_ignored_with_a_reason() {
        let h = hook_harness(true);
        let body =
            br#"{"ref":"refs/heads/dev","after":"1111111111111111111111111111111111111111"}"#;
        let response = h
            .app
            .oneshot(hook_request(
                "/hooks/github/web",
                body,
                ("x-hub-signature-256", github_header(body)),
            ))
            .await
            .expect("send");

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert!(json["ignored"].as_str().unwrap_or("").contains("dev"));
        let calls = h.exec.calls();
        assert_eq!(calls.len(), 1, "branch mismatch must not fetch: {calls:?}");
        assert!(calls[0].1.iter().any(|arg| arg == "symbolic-ref"));
    }

    #[tokio::test]
    async fn a_gitlab_push_with_the_right_token_deploys() {
        let h = hook_harness(true);
        let body =
            br#"{"ref":"refs/heads/main","after":"1111111111111111111111111111111111111111"}"#;
        let response = h
            .app
            .oneshot(hook_request(
                "/hooks/gitlab/web",
                body,
                ("x-gitlab-token", "s3cret".into()),
            ))
            .await
            .expect("send");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(body_json(response).await.get("deploy_id").is_some());
        assert!(h
            .exec
            .calls()
            .iter()
            .any(|(program, args)| { program == "git" && args.iter().any(|arg| arg == "reset") }));
    }

    #[tokio::test]
    async fn a_busy_push_is_ignored_without_reset_and_recorded() {
        let h = hook_harness(true);
        let active = h.store.create_deploy("web").expect("active deploy");
        assert!(h.store.acquire_lock("web", active).expect("lock"));
        let body =
            br#"{"ref":"refs/heads/main","after":"1111111111111111111111111111111111111111"}"#;

        let response = h
            .app
            .oneshot(hook_request(
                "/hooks/github/web",
                body,
                ("x-hub-signature-256", github_header(body)),
            ))
            .await
            .expect("send");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["ignored"], "deploy in progress");
        assert!(!h
            .exec
            .calls()
            .iter()
            .any(|(_, args)| args.iter().any(|arg| arg == "reset")));
        let attempts = h.store.recent_deploys("web", 10).expect("history");
        assert_eq!(attempts[0].status, DeployStatus::Failed);
        assert!(attempts[0]
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("hook ignored"));
    }

    #[tokio::test]
    async fn a_fetch_failure_is_redacted_and_recorded() {
        let h = hook_harness(true);
        h.exec.expect_call(
            "git",
            &["-C", &h.repo, "fetch", "origin"],
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "remote https://user:token@example.invalid failed".into(),
            },
        );
        let body =
            br#"{"ref":"refs/heads/main","after":"1111111111111111111111111111111111111111"}"#;

        let response = h
            .app
            .oneshot(hook_request(
                "/hooks/github/web",
                body,
                ("x-hub-signature-256", github_header(body)),
            ))
            .await
            .expect("send");

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let response_body = body_json(response).await.to_string();
        assert!(
            !response_body.contains("token"),
            "secret-bearing stderr leaked"
        );
        let attempts = h.store.recent_deploys("web", 10).expect("history");
        assert_eq!(attempts[0].status, DeployStatus::Failed);
        assert_eq!(attempts[0].detail.as_deref(), Some("hook git fetch failed"));
        assert_eq!(h.store.events_for(attempts[0].id).expect("events").len(), 1);
    }

    struct BlockingGitExecutor {
        fetch_started: tokio::sync::Notify,
        release_fetch: tokio::sync::Notify,
        calls: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    }

    impl BlockingGitExecutor {
        fn new() -> Self {
            Self {
                fetch_started: tokio::sync::Notify::new(),
                release_fetch: tokio::sync::Notify::new(),
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().expect("calls lock").clone()
        }
    }

    impl Executor for BlockingGitExecutor {
        fn run<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            program: &'life1 str,
            args: &'life2 [String],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = anyhow::Result<CommandOutput>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            self.calls
                .lock()
                .expect("calls lock")
                .push((program.to_string(), args.to_vec()));
            if program == "git" && args.iter().any(|arg| arg == "fetch") {
                self.fetch_started.notify_one();
                let release = &self.release_fetch;
                return Box::pin(async move {
                    release.notified().await;
                    Ok(ok(""))
                });
            }
            let stdout = if program == "git" && args.iter().any(|arg| arg == "symbolic-ref") {
                "main\n"
            } else if program == "systemctl" {
                "active\n"
            } else {
                ""
            };
            Box::pin(async move { Ok(ok(stdout)) })
        }
    }

    struct PendingExecutor;

    impl Executor for PendingExecutor {
        fn run<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _program: &'life1 str,
            _args: &'life2 [String],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = anyhow::Result<CommandOutput>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(std::future::pending())
        }
    }

    #[tokio::test]
    async fn a_hook_serializes_reset_reservation_and_reregistration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo-a");
        std::fs::create_dir_all(&repo).expect("repo dir");
        std::fs::write(
            repo.join("kuadrat.json"),
            serde_json::to_string(&WorkloadSpec::new("web", "placeholder")).expect("spec"),
        )
        .expect("write spec");
        let repo = repo.to_string_lossy().into_owned();
        let store = Arc::new(Store::open(&dir.path().join("k.db")).expect("store"));
        store
            .register_app(&AppConfig {
                name: "web".into(),
                repo_path: repo.clone(),
                route: None,
            })
            .expect("register");
        let exec = Arc::new(BlockingGitExecutor::new());
        let mut state = AppState::new(
            exec.clone(),
            Arc::new(FakeFileSystem::new()),
            store,
            Paths::rooted(dir.path()),
        );
        state.hook_secret = Some(Arc::new(secret()));
        let app = router(state.clone());
        // Keep the accepted deploy alive after the hook responds, so the API
        // trigger deterministically observes its durable in-progress row.
        let deploy_permit = state
            .deploy_slot
            .clone()
            .acquire_owned()
            .await
            .expect("slot");

        let body =
            br#"{"ref":"refs/heads/main","after":"1111111111111111111111111111111111111111"}"#;
        let hook = tokio::spawn(app.clone().oneshot(hook_request(
            "/hooks/github/web",
            body,
            ("x-hub-signature-256", github_header(body)),
        )));
        exec.fetch_started.notified().await;

        let api = tokio::spawn(
            app.clone().oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/apps/web/deploy")
                    .body(Body::empty())
                    .expect("api request"),
            ),
        );
        let new_repo = dir.path().join("repo-b").to_string_lossy().into_owned();
        let registration = tokio::spawn(
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/apps")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name":"web", "repo_path":new_repo}).to_string(),
                    ))
                    .expect("registration request"),
            ),
        );
        tokio::task::yield_now().await;
        assert!(!api.is_finished(), "API trigger bypassed the hook guard");
        assert!(
            !registration.is_finished(),
            "registration changed while the hook held its snapshot"
        );

        exec.release_fetch.notify_waiters();
        let hook_response = hook.await.expect("hook task").expect("hook response");
        let api_response = api.await.expect("api task").expect("api response");
        let registration_response = registration
            .await
            .expect("registration task")
            .expect("registration response");
        assert_eq!(hook_response.status(), StatusCode::OK);
        assert_eq!(api_response.status(), StatusCode::CONFLICT);
        assert_eq!(registration_response.status(), StatusCode::CREATED);

        let resets: Vec<_> = exec
            .calls()
            .into_iter()
            .filter(|(program, args)| program == "git" && args.iter().any(|arg| arg == "reset"))
            .collect();
        assert_eq!(resets.len(), 1, "only the winning hook resets: {resets:?}");
        assert_eq!(resets[0].1[1], repo, "reset used a changed registration");

        drop(deploy_permit);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hung_git_command_has_a_server_side_deadline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = AppState::new(
            Arc::new(PendingExecutor),
            Arc::new(FakeFileSystem::new()),
            Arc::new(Store::open(&dir.path().join("k.db")).expect("store")),
            Paths::rooted(dir.path()),
        );

        let err = run_git(
            &state,
            "web",
            "/srv/web",
            &["fetch", "origin"],
            "fetching origin",
        )
        .await
        .expect_err("the deadline must expire");

        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(body_json(response).await["error"]
            .as_str()
            .unwrap_or("")
            .contains("timed out"));
    }
}
