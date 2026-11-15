//! Test support crate. Integration tests live in `tests/`; this library only
//! exists so Cargo has a target to attach them to.

/// Environment variable consulted by `gguf-engine-config` for the data root.
pub const DATA_DIR_ENV: &str = gguf_engine_config::paths::DATA_DIR_ENV;
