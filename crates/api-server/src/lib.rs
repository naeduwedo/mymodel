//! Local REST and OpenAI-compatible API gateway (spec §9.2).
//!
//! # Security defaults
//!
//! These are not optional, and each is enforced in code rather than documented:
//!
//! * **loopback bind by default** - `127.0.0.1`, never `0.0.0.0`;
//! * **a bearer token is mandatory for any non-loopback bind** - the
//!   configuration is refused before a socket is opened;
//! * **no CORS headers unless the operator opts in** - a local model endpoint
//!   reachable from any web page is a confused-deputy hole;
//! * **no endpoint that is not backed by a capability** - `/v1/models` reads the
//!   real registry and `/v1/chat/completions` relays the real inference stream.
//!
//! # One inference path
//!
//! The gateway translates OpenAI JSON and relays the core's token stream. It does
//! not load models, so there is exactly one inference implementation in the
//! product instead of two that can drift apart.

#![warn(missing_docs)]

pub mod backend;
pub mod config;
pub mod error;
pub mod media;
pub mod openai;
pub mod routes;
pub mod server;

pub use backend::{Backend, ChunkStream, IpcBackend};
pub use config::{ApiConfig, ConfigError, DEFAULT_PORT, Exposure, MIN_TOKEN_LEN};
pub use error::ApiError;
pub use media::{LocalMedia, MediaBackend, image_response, parse_size};
pub use openai::{
    ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse, ChatMessage, ModelEntry,
    ModelList,
};
pub use routes::{AppState, router};
pub use server::{ApiServer, serve};

/// Whether this module is implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Implemented and tested.
    Implemented,
    /// Not implemented yet.
    Planned,
}

/// Current status of the crate.
pub const STATUS: Status = Status::Implemented;
