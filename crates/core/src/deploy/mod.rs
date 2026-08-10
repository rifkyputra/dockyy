//! Deploy value types. The state machine that uses them lands in G4.

/// A stage of the deploy loop. Stored as TEXT so the database is readable
/// with the `sqlite3` CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Detect,
    Build,
    Secrets,
    Apply,
    Route,
    Healthcheck,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Detect => "detect",
            Stage::Build => "build",
            Stage::Secrets => "secrets",
            Stage::Apply => "apply",
            Stage::Route => "route",
            Stage::Healthcheck => "healthcheck",
        }
    }

    // Inherent `from_str` returning Option, not the `FromStr` trait (which
    // returns Result). The store wants an Option to `.ok_or_else` on.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Stage> {
        match s {
            "detect" => Some(Stage::Detect),
            "build" => Some(Stage::Build),
            "secrets" => Some(Stage::Secrets),
            "apply" => Some(Stage::Apply),
            "route" => Some(Stage::Route),
            "healthcheck" => Some(Stage::Healthcheck),
            _ => None,
        }
    }
}

/// Terminal or in-flight status of a whole deploy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployStatus {
    InProgress,
    Done,
    RolledBack,
    Failed,
}

impl DeployStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeployStatus::InProgress => "in_progress",
            DeployStatus::Done => "done",
            DeployStatus::RolledBack => "rolled_back",
            DeployStatus::Failed => "failed",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<DeployStatus> {
        match s {
            "in_progress" => Some(DeployStatus::InProgress),
            "done" => Some(DeployStatus::Done),
            "rolled_back" => Some(DeployStatus::RolledBack),
            "failed" => Some(DeployStatus::Failed),
            _ => None,
        }
    }
}

use crate::exec::Executor;
use crate::fs::FileSystem;
use crate::store::Store;
use crate::workloads::paths::Paths;

/// A deploy failure, tagged with the stage it happened in.
#[derive(Debug, thiserror::Error)]
#[error("deploy failed at {stage:?}: {message}")]
pub struct DeployError {
    pub stage: Stage,
    pub message: String,
}

/// The terminal state of a deploy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployOutcome {
    /// Healthcheck passed. The only success.
    Done { image: String },
    /// A stage failed and compensation restored the previous version.
    RolledBack { failed_at: Stage, cause: String },
    /// A stage failed and compensation ALSO failed — host state is unknown.
    Failed { failed_at: Stage, cause: String },
}

/// Everything a deploy stage needs, bundled so stages take one argument.
pub struct Ctx<'a> {
    pub exec: &'a dyn Executor,
    pub fsys: &'a dyn FileSystem,
    pub store: &'a Store,
    pub paths: &'a Paths,
}

pub mod build;
pub mod detect;
pub mod health;
mod run;

pub use run::run;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_round_trips_through_its_string_form() {
        for stage in [
            Stage::Detect,
            Stage::Build,
            Stage::Secrets,
            Stage::Apply,
            Stage::Route,
            Stage::Healthcheck,
        ] {
            assert_eq!(Stage::from_str(stage.as_str()), Some(stage));
        }
    }

    #[test]
    fn stage_rejects_an_unknown_string() {
        assert_eq!(Stage::from_str("nonsense"), None);
    }

    #[test]
    fn deploy_status_round_trips() {
        for status in [
            DeployStatus::InProgress,
            DeployStatus::Done,
            DeployStatus::RolledBack,
            DeployStatus::Failed,
        ] {
            assert_eq!(DeployStatus::from_str(status.as_str()), Some(status));
        }
    }

    #[test]
    fn deploy_error_names_its_stage() {
        let err = DeployError {
            stage: Stage::Healthcheck,
            message: "timed out".into(),
        };
        let shown = err.to_string();
        assert!(shown.contains("Healthcheck"), "shown: {shown}");
        assert!(shown.contains("timed out"), "shown: {shown}");
    }

    #[test]
    fn deploy_outcome_variants_carry_their_data() {
        let done = DeployOutcome::Done {
            image: "localhost/kuadrat-web:abc".into(),
        };
        let rolled = DeployOutcome::RolledBack {
            failed_at: Stage::Route,
            cause: "caddy".into(),
        };
        assert_ne!(done, rolled);
        if let DeployOutcome::RolledBack { failed_at, .. } = rolled {
            assert_eq!(failed_at, Stage::Route);
        } else {
            panic!("wrong variant");
        }
    }
}
