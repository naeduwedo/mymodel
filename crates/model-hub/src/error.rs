//! Hub errors.

use thiserror::Error;

/// Why a Hub operation failed.
#[derive(Debug, Error)]
pub enum HubError {
    /// The HTTP request itself failed.
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),

    /// The Hub answered with a non-success status.
    #[error("the Hub answered {status} for {url}")]
    Status {
        /// Status code.
        status: u16,
        /// URL requested.
        url: String,
    },

    /// The Hub requires authentication for this repository.
    #[error("this repository is private or gated; provide an access token")]
    Unauthorized,

    /// The response body did not match the documented shape.
    #[error("unexpected Hub response: {0}")]
    Decode(#[from] serde_json::Error),

    /// A path would escape the destination directory.
    #[error("refusing to write outside the destination: {0}")]
    PathEscape(String),

    /// A file is larger than the free space allows.
    #[error("not enough disk space: need {needed} bytes, {available} available")]
    NoSpace {
        /// Bytes required.
        needed: u64,
        /// Bytes available.
        available: u64,
    },

    /// The downloaded bytes do not match the expected hash.
    #[error("checksum mismatch for {path}: expected {expected}, computed {actual}")]
    Checksum {
        /// File that failed.
        path: String,
        /// Hash the repository published.
        expected: String,
        /// Hash of what we received.
        actual: String,
    },

    /// A file has no published hash, so it cannot be verified.
    #[error("{path} has no published SHA-256; refusing to mark it ready")]
    Unverifiable {
        /// File without a hash.
        path: String,
    },

    /// Filesystem failure while staging or moving a file.
    #[error("filesystem error at {path}: {source}")]
    Io {
        /// Path involved.
        path: String,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },

    /// The operation was cancelled by the caller.
    #[error("cancelled")]
    Cancelled,
}
