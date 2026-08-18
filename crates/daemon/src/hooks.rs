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
use hmac::{Hmac, Mac};
use sha2::Sha256;

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
            return Ok(Some(Self(secret)));
        }
        if let Ok(path) = std::env::var("KUADRAT_HOOK_SECRET_FILE") {
            let contents = std::fs::read_to_string(&path)
                .with_context(|| format!("reading hook secret from {path}"))?;
            return Ok(Some(Self(contents.trim().to_string())));
        }
        Ok(None)
    }

    #[cfg(test)]
    pub fn for_tests(secret: &str) -> Self {
        Self(secret.to_string())
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
    if sha.is_empty() || sha.bytes().all(|b| b == b'0') {
        return None;
    }
    Some(Push { branch, sha })
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

    fn secret() -> HookSecret {
        HookSecret::for_tests("s3cret")
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
        // Garbage is not a push.
        assert!(parse_push(b"not json").is_none());
    }
}
