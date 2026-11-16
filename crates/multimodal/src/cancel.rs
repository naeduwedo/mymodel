//! A stop signal shared between a caller and the adapter doing the work.
//!
//! Renders take minutes. Both runtimes can be told to stop - sd.cpp through its
//! job API, audio.cpp by ending the child process - so the only thing missing was
//! a way for the caller to say "stop".

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A cheap, cloneable cancellation flag.
#[derive(Debug, Clone, Default)]
pub struct CancelHandle {
    cancelled: Arc<AtomicBool>,
}

impl CancelHandle {
    /// A handle that has not been cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the worker to stop. Idempotent.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether a stop has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Cancels the shared handle when dropped.
///
/// Wrapping a stream's producer in this makes "the client hung up" and "the user
/// pressed stop" the same event, which is what the runtime needs to hear.
#[derive(Debug)]
pub struct CancelOnDrop(pub CancelHandle);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_handle_is_not_cancelled() {
        assert!(!CancelHandle::new().is_cancelled());
    }

    #[test]
    fn cancelling_is_visible_to_every_clone() {
        let handle = CancelHandle::new();
        let clone = handle.clone();
        clone.cancel();
        assert!(handle.is_cancelled());
    }

    #[test]
    fn dropping_the_guard_cancels() {
        let handle = CancelHandle::new();
        {
            let _guard = CancelOnDrop(handle.clone());
            assert!(!handle.is_cancelled());
        }
        assert!(handle.is_cancelled());
    }

    #[test]
    fn cancelling_twice_is_harmless() {
        let handle = CancelHandle::new();
        handle.cancel();
        handle.cancel();
        assert!(handle.is_cancelled());
    }
}
