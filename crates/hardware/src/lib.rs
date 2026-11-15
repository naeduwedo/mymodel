//! Hardware inventory for GGUF AI Engine (spec §4.2, §5.2, §5.3, Task 4).
//!
//! # What this crate guarantees
//!
//! * Every value is either measured or explicitly unknown — never zero-filled
//!   to look like a measurement.
//! * Every measurement carries provenance: which probe produced it, at what time,
//!   with which tool version.
//! * P2P is reported as detected / driver-supported / tested, in that order of
//!   strength, and "tested" stays `Unsupported` until a benchmark proves it.
//! * A probe that cannot run says *why* in words the user can act on.
//!
//! # What it deliberately does not do
//!
//! * No `unsafe` vendor FFI: NVIDIA goes through `nvidia-smi`, AMD through
//!   `rocm-smi`, Apple through `system_profiler`. That costs a process spawn and
//!   buys a codebase with no driver-lifetime hazards (see ADR-013).
//! * No memory budgeting: that is Task 7. This crate answers "what is on this
//!   machine", not "what may I use".

#![warn(missing_docs)]

pub mod gpu_amd;
pub mod gpu_apple;
pub mod gpu_nvidia;
pub mod host;
pub mod orchestrate;
pub mod runner;
pub mod types;

/// Test doubles for probe tests.
#[cfg(test)]
pub mod testing;

pub use orchestrate::{collect, collect_with};
pub use runner::{CommandOutput, CommandRunner, SystemRunner};
pub use types::{
    CapabilityStatus, ComputeBackend, CpuInfo, GpuDevice, GpuTopology, GpuVendor, HostSnapshot,
    Interconnect, MemoryInfo, PeerLink, ProbeOutcome, Provenance, StorageInfo, unix_millis,
};
