//! IPC failures.

use thiserror::Error;

/// Errors raised by the IPC layer.
#[derive(Debug, Error)]
pub enum IpcError {
    /// The endpoint could not be bound.
    #[error("binding the IPC endpoint {endpoint}: {source}")]
    Bind {
        /// Endpoint name or socket path.
        endpoint: String,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The transport failed while serving or connecting.
    #[error("IPC transport error: {0}")]
    Transport(#[from] std::io::Error),
    /// The endpoint name or socket path is not acceptable.
    #[error("invalid IPC endpoint: {0}")]
    Endpoint(String),
    /// Access control could not be applied.
    #[error(transparent)]
    Security(#[from] gguf_engine_security::SecurityError),
    /// The client and core disagree about the protocol, or the core refused
    /// the handshake.
    #[error("handshake rejected: {0}")]
    Handshake(String),
    /// The call failed on the wire.
    #[error("gRPC call failed: {0}")]
    Rpc(#[from] tonic::Status),
    /// The core is not running at the expected endpoint.
    #[error("no core is listening at {0}; start it with `gguf-core serve`")]
    NotRunning(String),
    /// Any other error produced by a service implementation.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
