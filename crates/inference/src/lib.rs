//! Inference task orchestration (spec §9.1).
//!
//! # Status: planned (Task 6)
//!
//! Tasks move through
//! `queued -> preparing -> loading -> running -> postprocessing -> completed`
//! with `failed` / `cancelled` reachable from every non-terminal state. Every
//! task owns its resource plan and releases it on completion or failure.

#![warn(missing_docs)]

/// Whether this module is implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Implemented and tested.
    Implemented,
    /// Not implemented yet.
    Planned,
}

/// Current status of the crate.
pub const STATUS: Status = Status::Planned;
