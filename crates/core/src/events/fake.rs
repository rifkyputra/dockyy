use std::sync::Mutex;

use crate::deploy::Stage;
use crate::events::{EventSink, EventStatus, StoredEvent};

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

    /// `(stage, status)` pairs in order — the shape most deploy tests assert
    /// on, where the ids and details are noise.
    pub fn timeline(&self) -> Vec<(Stage, EventStatus)> {
        self.events()
            .iter()
            .map(|e| (e.event.stage, e.event.status))
            .collect()
    }
}

impl EventSink for FakeSink {
    fn emit(&self, event: &StoredEvent) {
        self.events.lock().expect("sink lock").push(event.clone());
    }
}
