//! Typed deploy events. G1 defines the type and its status; the store persists
//! them (Task 7). H1 adds live emission: [`EventSink`], the third seam, and
//! [`StoredEvent`], the read-side event that carries the id and insert time
//! the store assigned it.

pub mod fake;
pub mod null;

use crate::deploy::{DeployStatus, Stage};

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

/// The stage-column literal that marks a deploy-level event.
///
/// Not one of the six stage names, and `Stage::from_str` returns `None` for
/// it, so a reader that predates deploy-level events fails loudly on such a
/// row rather than misreading it as a stage.
pub const DEPLOY_ROW: &str = "deploy";

/// What an event is about: one stage of the deploy loop, or the deploy as a
/// whole reaching its terminal status.
///
/// The second variant exists because the `deploys` table and the event log
/// used to disagree about where a deploy's story ends. The table recorded
/// `Done`/`RolledBack`/`Failed`; the log stopped at the last stage event, so a
/// finished deploy was indistinguishable from one that stalled, and a rollback
/// that *succeeded* was invisible — a watcher saw "Apply failed" and silence.
/// The SSE stream closes on this variant, which is why it is a stored event
/// rather than a flag the handler computes.
///
/// `Finished` carries a `DeployStatus`, which can also spell `InProgress`.
/// That looseness is inherited deliberately from `Store::finish_deploy`, which
/// has the same signature — one vocabulary for one concept beats a second
/// enum and a conversion at every call site. The read path rejects a
/// non-terminal deploy-level row, so a corrupt one cannot be mistaken for a
/// real ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Stage { stage: Stage, status: EventStatus },
    Finished { status: DeployStatus },
}

impl EventKind {
    /// The `(stage, status)` column pair this kind is stored as — and, because
    /// the daemon's wire type uses the same projection, the pair a client
    /// sees. Having one function answer both questions is what keeps the
    /// database spelling and the JSON spelling from drifting apart.
    pub fn columns(&self) -> (&'static str, &'static str) {
        match self {
            EventKind::Stage { stage, status } => (stage.as_str(), status.as_str()),
            EventKind::Finished { status } => (DEPLOY_ROW, status.as_str()),
        }
    }
}

/// One durable event in a deploy's timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub deploy_id: i64,
    pub kind: EventKind,
    pub detail: Option<String>,
}

impl Event {
    /// An event about one stage of the deploy loop.
    pub fn for_stage(
        deploy_id: i64,
        stage: Stage,
        status: EventStatus,
        detail: Option<String>,
    ) -> Self {
        Self {
            deploy_id,
            kind: EventKind::Stage { stage, status },
            detail,
        }
    }

    /// An event about the deploy as a whole reaching a terminal status.
    pub fn finished(deploy_id: i64, status: DeployStatus, detail: Option<String>) -> Self {
        Self {
            deploy_id,
            kind: EventKind::Finished { status },
            detail,
        }
    }
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
        let ev = Event::for_stage(
            7,
            Stage::Build,
            EventStatus::Failed,
            Some("image build failed".into()),
        );
        assert_eq!(ev.deploy_id, 7);
        assert_eq!(
            ev.kind,
            EventKind::Stage {
                stage: Stage::Build,
                status: EventStatus::Failed
            }
        );
        assert_eq!(ev.detail.as_deref(), Some("image build failed"));
    }

    fn stored(id: i64, stage: Stage, status: EventStatus) -> StoredEvent {
        StoredEvent {
            id,
            at: "2026-01-01 00:00:00".into(),
            event: Event::for_stage(1, stage, status, None),
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
        assert_eq!(
            seen[1].event.kind,
            EventKind::Stage {
                stage: Stage::Build,
                status: EventStatus::Succeeded
            }
        );
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
