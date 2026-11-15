//! llama.cpp runtime adapter (spec 8.3).
//!
//! This crate integrates an installed llama.cpp build; it never reimplements
//! kernels (ADR-003). What it does:
//!
//! * probe - locate llama-server, detect backends and CUDA devices;
//! * capabilities - report what the binary can actually do;
//! * load - spawn llama-server for a model and wait for readiness;
//! * chat - stream tokens over its /completion endpoint;
//! * unload - stop the worker process.

#![warn(missing_docs)]

pub mod adapter;
pub mod client;
pub mod probe;
pub mod process;

pub use adapter::LlamaCppAdapter;
pub use probe::{BINARY_ENV, DEFAULT_SEARCH_DIRS, ProbeResult, locate};
