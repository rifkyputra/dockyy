//! Typed deploy events. G1 defines the type and its status; the store persists
//! them (Task 7). Live emission (a subscriber channel) arrives with the daemon
//! in phase 3.

use crate::deploy::Stage;

/// What happened to a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventStatus {
    Started,
    Succeeded,
    Failed,
}

impl EventStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventStatus::Started => "started",
            EventStatus::Succeeded => "succeeded",
            EventStatus::Failed => "failed",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<EventStatus> {
        match s {
            "started" => Some(EventStatus::Started),
            "succeeded" => Some(EventStatus::Succeeded),
            "failed" => Some(EventStatus::Failed),
            _ => None,
        }
    }
}

/// One durable event in a deploy's timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub deploy_id: i64,
    pub stage: Stage,
    pub status: EventStatus,
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::Stage;

    #[test]
    fn event_status_round_trips() {
        for status in [
            EventStatus::Started,
            EventStatus::Succeeded,
            EventStatus::Failed,
        ] {
            assert_eq!(EventStatus::from_str(status.as_str()), Some(status));
        }
    }

    #[test]
    fn an_event_carries_its_stage_and_detail() {
        let ev = Event {
            deploy_id: 7,
            stage: Stage::Build,
            status: EventStatus::Failed,
            detail: Some("image build failed".into()),
        };
        assert_eq!(ev.deploy_id, 7);
        assert_eq!(ev.stage, Stage::Build);
        assert_eq!(ev.status, EventStatus::Failed);
        assert_eq!(ev.detail.as_deref(), Some("image build failed"));
    }
}
