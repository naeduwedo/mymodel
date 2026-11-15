//! Thread-safe lifecycle owner plus the transition rules it enforces.
//!
//! ```text
//! Stopped -> Starting -> Ready <-> Degraded -> ShuttingDown -> Stopped
//! ```
//!
//! Transitions are validated in one place ([rules]) rather than scattered
//! across modules, so a UI can never observe an impossible sequence.
//!
//! The owner wraps the [rules] module, publishes
//! [`gguf_engine_event_bus::EventPayload::CoreStateChanged`] on every successful
//! transition and exposes a wait helper so a UI (or a test) can block until the
//! core is ready instead of polling.

use std::time::Duration;

use gguf_engine_event_bus::{CoreState, EventBus, EventPayload};
use parking_lot::Mutex;
use tokio::sync::watch;
use tracing::{debug, info, warn};

pub mod rules;

pub use rules::{TransitionError, allowed_successors, can_transition};

use rules::validate;

/// A successful transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateChange {
    /// State before the change.
    pub from: CoreState,
    /// State after the change.
    pub to: CoreState,
}

/// Owns the current [`CoreState`] and validates transitions.
#[derive(Debug)]
pub struct Lifecycle {
    state: Mutex<CoreState>,
    tx: watch::Sender<CoreState>,
    bus: EventBus,
}

impl Lifecycle {
    /// Create a lifecycle starting in [`CoreState::Stopped`].
    pub fn new(bus: EventBus) -> Self {
        let (tx, _rx) = watch::channel(CoreState::Stopped);
        Self {
            state: Mutex::new(CoreState::Stopped),
            tx,
            bus,
        }
    }

    /// Current state.
    pub fn current(&self) -> CoreState {
        *self.state.lock()
    }

    /// Watch channel for state changes.
    pub fn subscribe(&self) -> watch::Receiver<CoreState> {
        self.tx.subscribe()
    }

    /// Apply a transition.
    ///
    /// On success the change is published on the event bus; a failed publish
    /// (backpressure) is logged but does not roll the transition back — the
    /// state itself is authoritative and can be re-read via [`Self::current`].
    pub fn transition(&self, to: CoreState) -> Result<StateChange, TransitionError> {
        let mut guard = self.state.lock();
        let from = *guard;
        validate(from, to)?;
        *guard = to;
        drop(guard);

        info!(from = from.as_str(), to = to.as_str(), "core state changed");
        if let Err(err) = self
            .bus
            .publish(EventPayload::CoreStateChanged { from, to })
        {
            warn!(error = %err, "failed to publish core state change");
        }
        let _ = self.tx.send(to);
        Ok(StateChange { from, to })
    }

    /// Convenience: `Starting -> Ready`-style helper that validates the whole
    /// path and applies it step by step.
    pub fn boot(&self) -> Result<Vec<StateChange>, TransitionError> {
        Ok(vec![
            self.transition(CoreState::Starting)?,
            self.transition(CoreState::Ready)?,
        ])
    }

    /// Convenience: shutdown path `-> ShuttingDown -> Stopped`.
    pub fn shutdown(&self) -> Result<Vec<StateChange>, TransitionError> {
        let current = self.current();
        let mut changes = Vec::new();
        if current != CoreState::ShuttingDown {
            changes.push(self.transition(CoreState::ShuttingDown)?);
        }
        changes.push(self.transition(CoreState::Stopped)?);
        Ok(changes)
    }

    /// Wait until the state satisfies `predicate`, or time out.
    pub async fn wait_for(
        &self,
        predicate: impl Fn(CoreState) -> bool,
        timeout: Duration,
    ) -> Option<CoreState> {
        if predicate(self.current()) {
            return Some(self.current());
        }
        let mut rx = self.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                debug!("wait_for timed out");
                return None;
            }
            match tokio::time::timeout(remaining, rx.changed()).await {
                Ok(Ok(())) => {
                    let state = *rx.borrow_and_update();
                    if predicate(state) {
                        return Some(state);
                    }
                }
                Ok(Err(_)) => return None,
                Err(_) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gguf_engine_event_bus::BusConfig;

    fn lifecycle() -> Lifecycle {
        Lifecycle::new(EventBus::new(BusConfig::default()))
    }

    #[test]
    fn starts_stopped() {
        assert_eq!(lifecycle().current(), CoreState::Stopped);
    }

    #[test]
    fn boot_reaches_ready() {
        let lc = lifecycle();
        let changes = lc.boot().unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].from, CoreState::Stopped);
        assert_eq!(changes[0].to, CoreState::Starting);
        assert_eq!(changes[1].to, CoreState::Ready);
        assert_eq!(lc.current(), CoreState::Ready);
    }

    #[test]
    fn illegal_transition_is_rejected_and_state_is_unchanged() {
        let lc = lifecycle();
        let err = lc.transition(CoreState::Ready).unwrap_err();
        assert!(matches!(err, TransitionError::Illegal { .. }));
        assert_eq!(lc.current(), CoreState::Stopped);
    }

    #[test]
    fn transitions_are_published_on_the_bus() {
        let bus = EventBus::new(BusConfig::default());
        let lc = Lifecycle::new(bus.clone());
        lc.boot().unwrap();
        let replay = bus.replay_after(0);
        assert_eq!(replay.len(), 2);
        assert_eq!(
            replay[0].payload,
            EventPayload::CoreStateChanged {
                from: CoreState::Stopped,
                to: CoreState::Starting
            }
        );
        assert_eq!(
            replay[1].payload,
            EventPayload::CoreStateChanged {
                from: CoreState::Starting,
                to: CoreState::Ready
            }
        );
    }

    #[test]
    fn shutdown_passes_through_shutting_down() {
        let lc = lifecycle();
        lc.boot().unwrap();
        lc.transition(CoreState::Degraded).unwrap();
        let changes = lc.shutdown().unwrap();
        assert_eq!(changes[0].to, CoreState::ShuttingDown);
        assert_eq!(changes[1].to, CoreState::Stopped);
        assert_eq!(lc.current(), CoreState::Stopped);
    }

    #[test]
    fn shutdown_from_stopped_is_rejected_by_the_state_machine() {
        let lc = lifecycle();
        let err = lc.shutdown().unwrap_err();
        assert!(matches!(err, TransitionError::Illegal { .. }), "{err:?}");
        assert_eq!(lc.current(), CoreState::Stopped);
    }

    #[tokio::test]
    async fn wait_for_returns_immediately_when_already_satisfied() {
        let lc = lifecycle();
        lc.boot().unwrap();
        let state = lc
            .wait_for(|s| s == CoreState::Ready, Duration::from_millis(10))
            .await;
        assert_eq!(state, Some(CoreState::Ready));
    }

    #[tokio::test]
    async fn wait_for_times_out() {
        let lc = std::sync::Arc::new(lifecycle());
        let state = lc
            .wait_for(|s| s == CoreState::Ready, Duration::from_millis(30))
            .await;
        assert_eq!(state, None);
    }

    #[tokio::test]
    async fn wait_for_observes_later_transitions() {
        let lc = std::sync::Arc::new(lifecycle());
        let waiter = {
            let lc = lc.clone();
            tokio::spawn(async move {
                lc.wait_for(|s| s == CoreState::Ready, Duration::from_secs(2))
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        lc.transition(CoreState::Starting).unwrap();
        lc.transition(CoreState::Ready).unwrap();
        assert_eq!(waiter.await.unwrap(), Some(CoreState::Ready));
    }
}
