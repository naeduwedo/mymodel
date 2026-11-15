//! GGUF AI Engine core: lifecycle owner, policy and resource ledger.
//!
//! This crate deliberately contains **no** UI code and **no** inference kernels
//! (ADR-001, ADR-003). It owns:
//!
//! * the lifecycle state machine and single-instance lock (spec §3.3);
//! * the typed event bus shared with native UIs (spec §3.2);
//! * health reporting used by the CLI, the API and the UIs.

#![warn(missing_docs)]

pub mod health;
pub mod lifecycle;
pub mod lock;

pub use health::{ComponentHealth, HealthMonitor, HealthReport};
pub use lifecycle::{Lifecycle, StateChange, TransitionError, allowed_successors, can_transition};
pub use lock::{InstanceLock, LockError};

/// Name reported by the health endpoint.
pub const CORE_NAME: &str = "gguf-ai-engine-core";

/// Core version (from Cargo).
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
