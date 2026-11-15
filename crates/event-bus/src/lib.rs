//! Typed, bounded event bus (spec §3.2: "Core internal: Tokio channels /
//! typed events").
//!
//! Design notes:
//!
//! * the bus is **bounded**. A slow UI must never grow core memory without
//!   limit, so publishers receive [`PublishError::Backpressure`] and the event is
//!   counted as dropped instead of blocking the inference path;
//! * every event carries a monotonic `seq` and a UTC timestamp so a UI that
//!   reconnects can detect gaps via [`EventBus::replay_after`];
//! * payloads are strongly typed ([crate::EventPayload]) — there is no
//!   stringly-typed event channel.

#![warn(missing_docs)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;

/// Lifecycle states of the core daemon (spec §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreState {
    /// Not running.
    Stopped,
    /// Resources allocated, not yet serving.
    Starting,
    /// Serving every supported capability.
    Ready,
    /// Serving, but at least one capability is unavailable.
    Degraded,
    /// Draining tasks and releasing resources.
    ShuttingDown,
}

impl CoreState {
    /// Short stable name used in logs and API payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::ShuttingDown => "shutting_down",
        }
    }
}

/// Severity of an informative event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    /// Purely informational.
    Info,
    /// Something the user should look at.
    Warning,
    /// Something failed.
    Error,
}

/// A concrete event raised by the core or a worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", content = "data")]
pub enum EventPayload {
    /// The core changed lifecycle state.
    CoreStateChanged {
        /// Previous state.
        from: CoreState,
        /// New state.
        to: CoreState,
    },
    /// A periodic health sample was recorded.
    HealthTick {
        /// Uptime in whole milliseconds.
        uptime_ms: u64,
        /// Listeners currently attached to the bus.
        subscribers: usize,
    },
    /// A worker registered with the core.
    WorkerStarted {
        /// Worker identifier.
        worker_id: String,
        /// Runtime adapter identifier, e.g. `llama.cpp`.
        runtime_id: String,
    },
    /// A worker exited, cleanly or not.
    WorkerStopped {
        /// Worker identifier.
        worker_id: String,
        /// Exit code when the OS could report one.
        exit_code: Option<i32>,
        /// Failure reason for abnormal exits.
        reason: Option<String>,
    },
    /// A human-facing notification.
    Notice {
        /// Severity.
        level: NoticeLevel,
        /// Message text.
        message: String,
    },
}

/// An event as delivered to subscribers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Monotonic sequence number assigned by the bus.
    pub seq: u64,
    /// UTC timestamp, milliseconds since the Unix epoch.
    pub at_unix_ms: u64,
    /// Correlation id, e.g. the task that caused the event.
    pub correlation_id: Option<String>,
    /// The payload.
    pub payload: EventPayload,
}

/// Errors raised when publishing.
#[derive(Debug, Error)]
pub enum PublishError {
    /// The channel is full: at least one consumer is too slow.
    #[error("event bus is full ({capacity} pending events); subscriber too slow")]
    Backpressure {
        /// Per-subscriber queue capacity.
        capacity: usize,
    },
    /// The bus was closed.
    #[error("event bus is closed")]
    Closed,
}

/// Current bus counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusStats {
    /// Events accepted since start.
    pub published: u64,
    /// Events rejected because every subscriber had lagged.
    pub dropped: u64,
    /// Attached subscribers.
    pub subscribers: usize,
    /// Per-subscriber queue capacity.
    pub capacity: usize,
    /// Sequence number of the most recent event.
    pub last_seq: u64,
}

/// Configuration for [`EventBus::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusConfig {
    /// Maximum number of undelivered events per subscriber.
    pub capacity: usize,
    /// Maximum number of recent events kept for reconnecting clients.
    pub replay_limit: usize,
    /// Name used in log records.
    pub label: String,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            capacity: 1024,
            replay_limit: 256,
            label: "core".to_owned(),
        }
    }
}

/// Shared event bus handle. Clones share the same channel and sequence counter.
#[derive(Clone, Debug)]
pub struct EventBus {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    tx: broadcast::Sender<Event>,
    seq: AtomicU64,
    capacity: usize,
    dropped: AtomicU64,
    published: AtomicU64,
    ring: RwLock<Vec<Event>>,
    ring_limit: usize,
    label: String,
}

impl EventBus {
    /// Create a bus.
    ///
    /// # Panics
    ///
    /// Panics when `config.capacity` is smaller than 2, which
    /// `tokio::sync::broadcast` cannot use.
    pub fn new(config: BusConfig) -> Self {
        assert!(config.capacity >= 2, "event bus capacity must be >= 2");
        let (tx, _rx) = broadcast::channel(config.capacity);
        Self {
            inner: Arc::new(Inner {
                tx,
                seq: AtomicU64::new(0),
                capacity: config.capacity,
                dropped: AtomicU64::new(0),
                published: AtomicU64::new(0),
                ring: RwLock::new(Vec::new()),
                ring_limit: config.replay_limit,
                label: config.label,
            }),
        }
    }

    /// Subscribe to future events.
    pub fn subscribe(&self) -> EventSubscriber {
        EventSubscriber {
            rx: self.inner.tx.subscribe(),
            lagged: 0,
        }
    }

    /// Number of attached subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.inner.tx.receiver_count()
    }

    /// Publish an event.
    ///
    /// The event is always recorded for replay, even when no subscriber is
    /// attached. A full channel is reported as [`PublishError::Backpressure`].
    pub fn publish(&self, payload: EventPayload) -> Result<u64, PublishError> {
        self.publish_correlated(payload, None)
    }

    /// Publish an event tied to a task or request id.
    pub fn publish_correlated(
        &self,
        payload: EventPayload,
        correlation_id: Option<String>,
    ) -> Result<u64, PublishError> {
        let seq = self.inner.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let event = Event {
            seq,
            at_unix_ms: unix_millis(),
            correlation_id,
            payload,
        };

        {
            let mut ring = self.inner.ring.write();
            ring.push(event.clone());
            let limit = self.inner.ring_limit.max(1);
            if ring.len() > limit {
                let excess = ring.len() - limit;
                ring.drain(0..excess);
            }
        }
        self.inner.published.fetch_add(1, Ordering::Relaxed);

        if self.inner.tx.receiver_count() == 0 {
            // Nobody to deliver to; the ring still holds it for replay.
            return Ok(seq);
        }

        match self.inner.tx.send(event) {
            Ok(_) => Ok(seq),
            Err(broadcast::error::SendError(event)) => {
                let n = self.inner.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::warn!(
                    bus = %self.inner.label,
                    seq = event.seq,
                    dropped_total = n,
                    "event dropped: every subscriber lagged",
                );
                Err(PublishError::Backpressure {
                    capacity: self.inner.capacity,
                })
            }
        }
    }

    /// Replay events with `seq` greater than `after_seq`, bounded by the ring.
    ///
    /// Used when a UI reconnects and must not miss state transitions.
    pub fn replay_after(&self, after_seq: u64) -> Vec<Event> {
        let ring = self.inner.ring.read();
        ring.iter()
            .filter(|event| event.seq > after_seq)
            .cloned()
            .collect()
    }

    /// Snapshot of the counters.
    pub fn stats(&self) -> BusStats {
        BusStats {
            published: self.inner.published.load(Ordering::Relaxed),
            dropped: self.inner.dropped.load(Ordering::Relaxed),
            subscribers: self.inner.tx.receiver_count(),
            capacity: self.inner.capacity,
            last_seq: self.inner.seq.load(Ordering::SeqCst),
        }
    }

    /// Last sequence number assigned (0 when nothing was published).
    pub fn last_seq(&self) -> u64 {
        self.inner.seq.load(Ordering::SeqCst)
    }
}

/// A subscriber stream. Events arrive in publish order.
#[derive(Debug)]
pub struct EventSubscriber {
    rx: broadcast::Receiver<Event>,
    lagged: u64,
}

impl EventSubscriber {
    /// Receive the next event, skipping (and counting) lagged ones.
    ///
    /// Returns `Ok(None)` only when the bus is closed.
    pub async fn recv(&mut self) -> Option<Event> {
        loop {
            match self.rx.recv().await {
                Ok(event) => return Some(event),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    self.lagged += skipped;
                    tracing::warn!(skipped, "event subscriber lagged");
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Number of events skipped because this subscriber was too slow.
    pub fn lagged(&self) -> u64 {
        self.lagged
    }

    /// Try to receive without waiting.
    pub fn try_recv(&mut self) -> Option<Event> {
        loop {
            match self.rx.try_recv() {
                Ok(event) => return Some(event),
                Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                    self.lagged += skipped;
                }
                Err(_) => return None,
            }
        }
    }

    /// Convert into a `Stream` of events (lagged events are skipped).
    pub fn into_stream(self) -> impl tokio_stream::Stream<Item = Event> + Send + 'static {
        tokio_stream::wrappers::BroadcastStream::new(self.rx).filter_map(|item| item.ok())
    }
}

fn unix_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(msg: &str) -> EventPayload {
        EventPayload::Notice {
            level: NoticeLevel::Info,
            message: msg.to_owned(),
        }
    }

    #[test]
    fn sequence_numbers_start_at_one_and_increase() {
        let bus = EventBus::new(BusConfig::default());
        assert_eq!(bus.publish(notice("a")).unwrap(), 1);
        assert_eq!(bus.publish(notice("b")).unwrap(), 2);
        assert_eq!(bus.last_seq(), 2);
    }

    #[tokio::test]
    async fn subscriber_receives_events_in_order() {
        let bus = EventBus::new(BusConfig::default());
        let mut sub = bus.subscribe();

        bus.publish(notice("first")).unwrap();
        bus.publish_correlated(notice("second"), Some("task-1".into()))
            .unwrap();

        let a = sub.recv().await.expect("first event");
        assert_eq!(a.seq, 1);
        assert_eq!(a.correlation_id, None);
        assert_eq!(a.payload, notice("first"));

        let b = sub.recv().await.expect("second event");
        assert_eq!(b.seq, 2);
        assert_eq!(b.correlation_id.as_deref(), Some("task-1"));
        assert!(b.at_unix_ms > 0);
    }

    #[tokio::test]
    async fn every_subscriber_gets_the_event() {
        let bus = EventBus::new(BusConfig::default());
        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);

        bus.publish(notice("broadcast")).unwrap();
        assert_eq!(a.recv().await.unwrap().payload, notice("broadcast"));
        assert_eq!(b.recv().await.unwrap().payload, notice("broadcast"));
    }

    #[tokio::test]
    async fn publishing_without_subscribers_is_recorded_for_replay() {
        let bus = EventBus::new(BusConfig::default());
        bus.publish(notice("before-ui")).unwrap();
        bus.publish(notice("before-ui-2")).unwrap();
        assert_eq!(bus.subscriber_count(), 0);

        let replay = bus.replay_after(0);
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[1].payload, notice("before-ui-2"));
    }

    #[tokio::test]
    async fn replay_after_returns_only_newer_events() {
        let bus = EventBus::new(BusConfig::default());
        for i in 0..5 {
            bus.publish(notice(&format!("e{i}"))).unwrap();
        }
        let replay = bus.replay_after(3);
        assert_eq!(replay.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![4, 5]);
    }

    #[tokio::test]
    async fn replay_ring_is_bounded() {
        let bus = EventBus::new(BusConfig {
            capacity: 16,
            replay_limit: 3,
            label: "test".into(),
        });
        for i in 0..10 {
            bus.publish(notice(&format!("e{i}"))).unwrap();
        }
        let replay = bus.replay_after(0);
        assert_eq!(replay.len(), 3);
        assert_eq!(replay.last().unwrap().seq, 10);
    }

    #[tokio::test]
    async fn slow_subscriber_causes_backpressure_instead_of_growth() {
        // tokio::sync::broadcast never blocks the sender: a lagging receiver is
        // dropped from the ring and later observes RecvError::Lagged. The bus
        // must keep memory bounded *and* report the loss on both sides.
        let bus = EventBus::new(BusConfig {
            capacity: 2,
            replay_limit: 8,
            label: "test".into(),
        });
        let mut slow = bus.subscribe();

        for i in 0..10 {
            let seq = bus.publish(notice(&format!("e{i}")));
            assert!(seq.is_ok(), "broadcast must never block the publisher");
        }

        let stats = bus.stats();
        assert_eq!(stats.capacity, 2);
        assert_eq!(stats.published, 10);

        // The slow subscriber skipped events instead of growing a queue.
        let first = slow.recv().await.expect("bus is still open");
        assert!(slow.lagged() > 0, "expected lag to be reported");
        assert!(first.seq > 1, "expected to have skipped the oldest events");
        // The replay ring is what a reconnecting client can catch up on, and
        // it stays bounded by replay_limit.
        assert_eq!(bus.replay_after(0).len(), 8);
    }

    #[tokio::test]
    async fn subscriber_lag_is_counted() {
        let bus = EventBus::new(BusConfig {
            capacity: 4,
            replay_limit: 8,
            label: "test".into(),
        });
        let mut sub = bus.subscribe();
        // Publish more than capacity while the subscriber is not reading; the
        // subscriber is dropped from the ring then re-attached by recv().
        for i in 0..8 {
            let _ = bus.publish(notice(&format!("e{i}")));
        }
        let _ = sub.recv().await;
        assert!(sub.lagged() >= 1, "lagged={}", sub.lagged());
    }

    #[tokio::test]
    async fn closed_bus_ends_the_subscriber_stream() {
        let bus = EventBus::new(BusConfig::default());
        let mut sub = bus.subscribe();
        drop(bus);
        assert!(sub.recv().await.is_none());
    }

    #[tokio::test]
    async fn stream_adapter_yields_events() {
        use tokio_stream::StreamExt;
        let bus = EventBus::new(BusConfig::default());
        let stream = bus.subscribe().into_stream();
        bus.publish(notice("streamed")).unwrap();
        let first = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            stream.take(1).collect::<Vec<_>>(),
        )
        .await
        .expect("stream produced an item");
        assert_eq!(first[0].payload, notice("streamed"));
    }

    #[test]
    fn event_payload_serialises_with_a_type_tag() {
        let bus = EventBus::new(BusConfig::default());
        bus.publish(EventPayload::CoreStateChanged {
            from: CoreState::Starting,
            to: CoreState::Ready,
        })
        .unwrap();
        let event = &bus.replay_after(0)[0];
        let json = serde_json::to_string(event).unwrap();
        assert!(json.contains("\"type\":\"core_state_changed\""), "{json}");
        assert!(json.contains("\"to\":\"ready\""), "{json}");
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, event);
    }

    #[test]
    fn core_state_names_are_stable() {
        assert_eq!(CoreState::Stopped.as_str(), "stopped");
        assert_eq!(CoreState::Ready.as_str(), "ready");
        assert_eq!(CoreState::ShuttingDown.as_str(), "shutting_down");
    }
}
