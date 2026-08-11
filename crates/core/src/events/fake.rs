use std::sync::Mutex;

use crate::deploy::{DeployStatus, Stage};
use crate::events::{EventKind, EventSink, EventStatus, StoredEvent};

/// Records every emitted event so tests can assert on the timeline.
#[derive(Default)]
pub struct FakeSink {
    events: Mutex<Vec<StoredEvent>>,
}

impl FakeSink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every event emitted so far, in order.
    pub fn events(&self) -> Vec<StoredEvent> {
        self.events.lock().expect("sink lock").clone()
    }

    /// `(stage, status)` pairs in order, deploy-level events excluded — the
    /// shape most deploy tests assert on, where the ids and details are noise.
    /// The terminal event is asserted separately, via `terminal()`.
    pub fn timeline(&self) -> Vec<(Stage, EventStatus)> {
        self.events()
            .iter()
            .filter_map(|e| match e.event.kind {
                EventKind::Stage { stage, status } => Some((stage, status)),
                EventKind::Finished { .. } => None,
            })
            .collect()
    }

    /// The terminal status this deploy reported, if it has reported one.
    pub fn terminal(&self) -> Option<DeployStatus> {
        self.events().iter().find_map(|e| match e.event.kind {
            EventKind::Finished { status } => Some(status),
            EventKind::Stage { .. } => None,
        })
    }
}

impl EventSink for FakeSink {
    fn emit(&self, event: &StoredEvent) {
        self.events.lock().expect("sink lock").push(event.clone());
    }
}
