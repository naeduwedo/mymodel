//! Memory orchestration for VRAM, host RAM and NVMe (spec 6).
//!
//! # Invariants
//!
//! * NVMe is a cold tier and is never addressable like memory (ADR-006). The OS
//!   swap file is not used as a model paging mechanism at all.
//! * Every tier has a reserve; `usable = available - reserve`, saturating at zero.
//! * Prefetch is bounded by a hard cap and stops entirely under pressure.
//! * Engine-managed tensor paging stays off unless a runtime proves safe tensor
//!   lifetimes (ADR-005); this crate does not move tensors on its own.

#![warn(missing_docs)]

pub mod tiers;

pub use tiers::{
    PrefetchError, PrefetchRequest, Pressure, Tier, TierBudget, grant_prefetch, host_memory,
};
