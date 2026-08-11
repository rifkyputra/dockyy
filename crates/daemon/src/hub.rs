//! The broadcast hub: the daemon's `EventSink`, and the subscription every SSE
//! stream reads from.
//!
//! This is the daemon's answer to a question `core` deliberately does not ask.
//! `core` emits events into a trait object and knows nothing about who is
//! listening; this type is the "someone is listening" case, and it lives here
//! rather than in `core` because a pub/sub channel is transport machinery
//! (ADR-0002).

use kuadrat_core::events::{EventSink, StoredEvent};
use tokio::sync::broadcast;

/// How many events the channel holds for a receiver that has fallen behind,
/// before it starts dropping them and reporting `Lagged`.
///
/// A deploy emits thirteen events — six stages started and succeeded, plus the
/// terminal one — so this is several whole deploys of slack. It is a cushion,
/// not a guarantee: the stream recovers from a lag by re-reading SQLite, and
/// that path has to work regardless, so a larger buffer would only make it
/// rarer and therefore less tested.
const CAPACITY: usize = 256;

/// An `EventSink` that publishes every event to every live subscriber.
pub struct BroadcastSink {
    tx: broadcast::Sender<StoredEvent>,
}

impl BroadcastSink {
    pub fn new() -> Self {
        Self::with_capacity(CAPACITY)
    }

    /// A hub with a chosen buffer. Exists so a test can provoke `Lagged`
    /// deterministically instead of racing 256 events against a reader.
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// A receiver for every event emitted from this moment on.
    ///
    /// Callers must subscribe *before* reading the stored backlog. An event
    /// landing between the read and the subscribe would otherwise be lost, and
    /// it is exactly the stage transition the viewer is waiting for. In this
    /// order the event arrives twice instead, and the stream's id filter drops
    /// the duplicate.
    pub fn subscribe(&self) -> broadcast::Receiver<StoredEvent> {
        self.tx.subscribe()
    }
}

impl Default for BroadcastSink {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for BroadcastSink {
    fn emit(&self, event: &StoredEvent) {
        // `send` is synchronous and never blocks — it writes into a ring
        // buffer — which is what lets this satisfy `emit`'s contract of not
        // suspending the deploy loop.
        //
        // It errors only when there are no receivers at all. That is a daemon
        // nobody is watching, not a fault, and dropping it here is precisely
        // what the sink's infallible signature exists to guarantee.
        let _ = self.tx.send(event.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuadrat_core::deploy::Stage;
    use kuadrat_core::events::{Event, EventStatus};

    fn stored(id: i64) -> StoredEvent {
        StoredEvent {
            id,
            at: "2026-01-01 00:00:00".into(),
            event: Event::for_stage(1, Stage::Build, EventStatus::Started, None),
        }
    }

    #[tokio::test]
    async fn a_subscriber_receives_what_is_emitted_after_it_subscribed() {
        let hub = BroadcastSink::new();
        let mut rx = hub.subscribe();
        hub.emit(&stored(7));

        let got = rx.recv().await.expect("recv");
        assert_eq!(got.id, 7);
    }

    #[tokio::test]
    async fn every_subscriber_receives_every_event() {
        let hub = BroadcastSink::new();
        let mut a = hub.subscribe();
        let mut b = hub.subscribe();
        hub.emit(&stored(1));

        assert_eq!(a.recv().await.expect("a").id, 1);
        assert_eq!(b.recv().await.expect("b").id, 1);
    }

    /// The sink's signature promises a deploy cannot be failed by whoever is
    /// watching. Emitting into a hub with no subscribers must therefore be a
    /// no-op, not an error — and it is the normal state of a daemon nobody has
    /// a browser open against.
    #[tokio::test]
    async fn emitting_with_nobody_listening_is_not_an_error() {
        let hub = BroadcastSink::new();
        hub.emit(&stored(1));
    }

    /// A receiver that falls behind loses messages and is told so. The stream
    /// handler recovers by re-reading SQLite; this test pins the behaviour the
    /// recovery is written against.
    #[tokio::test]
    async fn a_slow_subscriber_is_told_it_lagged_rather_than_silently_skipping() {
        use tokio::sync::broadcast::error::RecvError;

        let hub = BroadcastSink::with_capacity(2);
        let mut rx = hub.subscribe();
        for id in 1..=5 {
            hub.emit(&stored(id));
        }
        assert!(matches!(rx.recv().await, Err(RecvError::Lagged(_))));
    }
}
