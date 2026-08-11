use crate::events::{EventSink, StoredEvent};

/// Discards every event. Used by the CLI's in-process paths, where nothing is
/// watching and the durable record in the store is the whole story.
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: &StoredEvent) {}
}
