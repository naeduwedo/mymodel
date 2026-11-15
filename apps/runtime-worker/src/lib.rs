//! Binary metadata for `gguf-engine-runtime-worker`.
//!
//! The crate's logic lives in the binary target; this library exists so
//! integration tests can depend on the package and receive the
//! `CARGO_BIN_EXE_<name>` environment variable from cargo.

/// Name of the produced executable.
pub const BINARY_NAME: &str = "gguf-worker";
