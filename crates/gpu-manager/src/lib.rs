//! GPU topology-aware scheduling and VRAM budgets (spec 5).
//!
//! # Invariants this crate enforces
//!
//! * VRAM is budgeted **per device**: `usable = free - engine_reserve - safety_margin`.
//!   Capacities of several GPUs are never summed into one pool (spec 5.4).
//! * An unsupported strategy is never proposed as a fallback: the planner only
//!   returns what the runtime and this hardware can actually execute.
//! * Every allocation takes a lease that expires, so a crashed worker cannot pin
//!   a GPU forever.

#![warn(missing_docs)]

pub mod lease;
pub mod planner;
pub mod types;

pub use lease::{GpuLedger, Lease, LeaseError, ReserveRequest};
pub use planner::{RuntimeFacts, plan, total_usable};
pub use types::{ExecutionPlan, ExecutionStrategy, GpuBudget, ModelMemoryRequirement, PlanError};
