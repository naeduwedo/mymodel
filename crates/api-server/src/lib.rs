//! Local REST and OpenAI-compatible API gateway (spec §9.2).
//!
//! # Status: planned (Task 2)
//!
//! Security defaults are not optional: loopback bind, explicit opt-in for LAN,
//! mandatory bearer token for non-loopback binds, restrictive CORS, and no
//! endpoint that is not actually backed by a capability.

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
