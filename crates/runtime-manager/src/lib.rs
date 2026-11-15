//! Runtime adapter trait, capability registry and worker supervision (spec 8).
//!
//! # What lives here
//!
//! * adapter::RuntimeAdapter - the one boundary every runtime implements;
//! * types - capability status, instances, requests and errors.
//!
//! The concrete llama.cpp implementation is gguf-engine-runtime-llama-cpp; this
//! crate owns the contract so image/video/audio adapters (Task 8) plug into the
//! same manager without churn.

#![warn(missing_docs)]

pub mod adapter;
pub mod types;

pub use adapter::{BoxedAdapter, LoadPlan, MemoryEstimate, RuntimeAdapter, TokenStream};
pub use types::{
    Capability, CapabilityStatus, ChatMessage, ChatRequest, RuntimeCapabilities, RuntimeError,
    RuntimeInstance, Token,
};
