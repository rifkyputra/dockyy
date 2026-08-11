//! Typed deploy events. G1 defines the type and its status; the store persists
//! them (Task 7). H1 adds live emission: [`EventSink`], the third seam, and
//! [`StoredEvent`], the read-side event that carries the id and insert time
//! the store assigned it.

pub mod fake;
pub mod null;

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

/// An event as it exists after the store has written it: the same event, plus
/// the id SQLite assigned.
///
/// The split from [`Event`] is deliberate. An id only exists after the insert,
/// so a type that carries one cannot be constructed before persisting — which
/// makes "persist first, then publish" a property of the types rather than a
/// rule someone has to remember. The SSE stream needs the id to deduplicate at
/// the join and to honour `Last-Event-ID`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    pub id: i64,
    /// The store's insert timestamp for this row (SQLite `datetime('now')`,
    /// UTC). Present only on the read side, for the same reason as `id`: a
    /// write-side [`Event`] has not been inserted yet, so it has no timestamp.
    pub at: String,
    pub event: Event,
}

/// The third seam, beside [`crate::exec::Executor`] and [`crate::fs::FileSystem`].
///
/// Publishes an already-persisted event to whoever is watching. Implementations:
/// `NullSink` (drops), `FakeSink` (records, for tests), and a broadcast-backed
/// sink in the daemon.
///
/// `emit` returns nothing and is synchronous, both deliberately. A subscriber
/// that has gone away must not be able to fail a deploy, so there is no error
/// to propagate; and a sink cannot await, so it cannot suspend the deploy
/// loop. It must also not block, either — a sink needing I/O hands off to a
/// channel and does that work in its own task.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &StoredEvent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::Stage;
    use crate::events::fake::FakeSink;
    use crate::events::null::NullSink;

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

    fn stored(id: i64, stage: Stage, status: EventStatus) -> StoredEvent {
        StoredEvent {
            id,
            at: "2026-01-01 00:00:00".into(),
            event: Event {
                deploy_id: 1,
                stage,
                status,
                detail: None,
            },
        }
    }

    #[test]
    fn fake_sink_records_every_event_in_order() {
        let sink = FakeSink::new();
        sink.emit(&stored(1, Stage::Build, EventStatus::Started));
        sink.emit(&stored(2, Stage::Build, EventStatus::Succeeded));

        let seen = sink.events();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].id, 1);
        assert_eq!(seen[1].event.status, EventStatus::Succeeded);
    }

    /// The shape most deploy tests assert on: which stages happened, and how
    /// each ended. Spelling that out per-test would bury the assertion.
    #[test]
    fn fake_sink_timeline_is_stage_status_pairs() {
        let sink = FakeSink::new();
        sink.emit(&stored(1, Stage::Detect, EventStatus::Started));
        sink.emit(&stored(2, Stage::Detect, EventStatus::Succeeded));
        sink.emit(&stored(3, Stage::Build, EventStatus::Failed));

        assert_eq!(
            sink.timeline(),
            vec![
                (Stage::Detect, EventStatus::Started),
                (Stage::Detect, EventStatus::Succeeded),
                (Stage::Build, EventStatus::Failed),
            ]
        );
    }

    #[test]
    fn null_sink_accepts_events_and_does_nothing() {
        let sink = NullSink;
        sink.emit(&stored(1, Stage::Apply, EventStatus::Started));
        // No panic, no state. The assertion is that this compiles and runs:
        // NullSink is what the CLI passes, and it must never fail a deploy.
    }

    /// Both implementations must be usable behind `&dyn EventSink`, because
    /// `Ctx` holds one that way.
    #[test]
    fn both_sinks_are_usable_as_trait_objects() {
        let fake = FakeSink::new();
        let sinks: Vec<&dyn EventSink> = vec![&fake, &NullSink];
        for sink in sinks {
            sink.emit(&stored(9, Stage::Route, EventStatus::Started));
        }
        assert_eq!(fake.events().len(), 1);
    }
}
