//! Platform-independent endpoint helpers.
//!
//! The transport itself lives in [`crate::serve`]; this module owns the naming
//! rules and the "is it alive?" probe so tests and the CLI agree with the
//! server.

use std::path::Path;

/// Endpoint for a data root (see [`gguf_engine_protocol::ipc_endpoint`]).
pub fn endpoint_for(data_root: &Path) -> String {
    gguf_engine_protocol::ipc_endpoint(data_root)
}

/// True when something is listening at the endpoint.
///
/// **Diagnostics only — never call this before connecting.** On Windows each
/// successful probe consumes the single listening pipe instance, which would
/// race with the real connection. [`crate::connect`] therefore connects
/// directly and interprets the failure instead.
///
/// * Windows: opening the pipe fails with ERROR_FILE_NOT_FOUND when no
///   instance exists;
/// * Unix: `connect` against the socket path fails with ECONNREFUSED/ENOENT.
pub async fn is_listening(endpoint: &str) -> bool {
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        ClientOptions::new().open(endpoint).is_ok()
    }
    #[cfg(unix)]
    {
        tokio::net::UnixStream::connect(endpoint).await.is_ok()
    }
    #[cfg(not(any(windows, unix)))]
    {
        false
    }
}
