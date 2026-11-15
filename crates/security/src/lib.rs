//! Access control for the local IPC surface (spec §3.2, §9.3).
//!
//! The core is a local service that can load multi-gigabyte models and saturate
//! the GPU, so "who may connect" is a security boundary, not a nicety:
//!
//! * on Windows the named pipe is created with a security descriptor that
//!   grants access to the current user and SYSTEM only;
//! * on Unix the socket lives in a 0700 directory;
//! * tokens are compared in constant time, so a timing side channel cannot be
//!   used to guess them.
//!
//! Nothing here logs a token (spec §9.3).

#![warn(missing_docs)]
// `unsafe` is forbidden everywhere else in the workspace. The only exception
// is [`windows_acl`]: Win32 has no safe wrapper for resolving the current
// user's SID, and the alternative — granting Everyone access to the pipe —
// would defeat the point of the check. Every block there carries a SAFETY
// note, and the exception is narrowed to that module. See
// docs/adr/ADR-010-unsafe-windows-pipe-acl.md.
#![deny(unsafe_code)]

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_acl;

/// Windows named-pipe creation with the user-only DACL applied.
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows_pipe;

use std::path::{Path, PathBuf};

use thiserror::Error;

/// Access-control failures.
#[derive(Debug, Error)]
pub enum SecurityError {
    /// The socket directory could not be secured.
    #[error("securing the IPC directory {path}: {source}")]
    Directory {
        /// Directory path.
        path: String,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The (Windows) security descriptor could not be applied.
    #[error("securing the IPC endpoint: {0}")]
    Endpoint(String),
    /// A connecting client is not the same user as the core.
    #[error("rejected IPC peer: {0}")]
    ForeignPeer(String),
    /// The peer identity could not be determined.
    #[error("could not determine the IPC peer identity: {0}")]
    PeerUnknown(String),
    /// A bearer token was missing or wrong.
    #[error("unauthorized: a valid API token is required")]
    Unauthorized,
}

/// A bearer token held by the core.
///
/// The value is never formatted, logged or compared with equality:
/// [`Token::matches`] runs a constant-time comparison even when the lengths
/// differ.
#[derive(Clone)]
pub struct Token(String);

impl Token {
    /// Wrap a token value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Constant-time comparison.
    pub fn matches(&self, presented: &str) -> bool {
        let a = self.0.as_bytes();
        let b = presented.as_bytes();
        // Walk the longer input and fold the length difference into the
        // accumulator, so the loop never exits early on a length mismatch.
        let mut diff = a.len() ^ b.len();
        let len = a.len().max(b.len()).max(1);
        for index in 0..len {
            let x = a.get(index).copied().unwrap_or(0);
            let y = b.get(index).copied().unwrap_or(0);
            diff |= usize::from(x ^ y);
        }
        diff == 0
    }

    /// Length in bytes (safe to log, unlike the value).
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when the token is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Token(<redacted, {} bytes>)", self.0.len())
    }
}

/// Ensure the directory holding the Unix socket is private to the user.
///
/// Windows has no equivalent (the pipe is protected by its descriptor), so the
/// function only creates the directory there.
pub fn secure_socket_directory(dir: &Path) -> Result<(), SecurityError> {
    std::fs::create_dir_all(dir).map_err(|source| SecurityError::Directory {
        path: dir.display().to_string(),
        source,
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let permissions = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(dir, permissions).map_err(|source| SecurityError::Directory {
            path: dir.display().to_string(),
            source,
        })?;
    }
    Ok(())
}

/// SDDL DACL applied to every pipe instance the core creates.
///
/// Exposed so the IPC layer can describe its endpoint in logs and tests without
/// depending on `windows` types; on non-Windows platforms it returns an empty
/// string because the protection is provided by the socket directory mode.
#[cfg(windows)]
pub fn pipe_acl_sddl() -> Result<String, SecurityError> {
    windows_acl::current_user_sddl()
}

/// See [`pipe_acl_sddl`]; Unix relies on the 0700 socket directory instead.
#[cfg(not(windows))]
pub fn pipe_acl_sddl() -> Result<String, SecurityError> {
    Ok(String::new())
}

/// Check that an endpoint name is usable for a local named pipe.
#[cfg(windows)]
pub fn validate_pipe_name(name: &str) -> Result<(), SecurityError> {
    windows_acl::validate_pipe_name(name)
}

/// Endpoint names on Unix are filesystem paths; only `\0` is invalid.
#[cfg(not(windows))]
pub fn validate_pipe_name(name: &str) -> Result<(), SecurityError> {
    if name.is_empty() || name.contains('\0') {
        return Err(SecurityError::Endpoint(format!(
            "invalid socket path: {name}"
        )));
    }
    Ok(())
}

/// Where the OS stores the peer-user information for a connection.
///
/// Not used directly here: peer identity on Unix is obtained with
/// `SO_PEERCRED`/`getpeereid` by the IPC listener, and on Windows the pipe ACL
/// rejects foreign users before a connection exists. The constant documents
/// that split so a reviewer can check it.
pub const PEER_CHECK_STRATEGY: &str = if cfg!(windows) {
    "pipe-dacl"
} else {
    "socket-mode"
};

/// Directory holding the Unix socket for a data root.
pub fn socket_dir(data_root: &Path) -> PathBuf {
    data_root.join("run")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_matches_only_the_exact_value() {
        let token = Token::new("s3cret-token");
        assert!(token.matches("s3cret-token"));
        assert!(!token.matches("s3cret-toke"));
        assert!(!token.matches("s3cret-token "));
        assert!(!token.matches(""));
        assert!(!token.matches("S3CRET-TOKEN"));
    }

    #[test]
    fn token_comparison_does_not_short_circuit_on_length() {
        // A token whose prefix matches must still be rejected; this guards
        // against a comparison that stops at the first differing length.
        let token = Token::new("abcd");
        assert!(!token.matches("abcdabcd"));
        assert!(!token.matches("abc"));
    }

    #[test]
    fn empty_token_only_matches_empty_input() {
        let token = Token::new("");
        assert!(token.matches(""));
        assert!(!token.matches("x"));
        assert!(token.is_empty());
    }

    #[test]
    fn debug_output_redacts_the_value() {
        let token = Token::new("hunter2");
        let rendered = format!("{token:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("7 bytes"), "{rendered}");
    }

    #[cfg(unix)]
    #[test]
    fn socket_directory_is_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("run");
        secure_socket_directory(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn socket_dir_is_the_run_subdirectory() {
        assert_eq!(
            socket_dir(Path::new("/data")),
            PathBuf::from("/data").join("run")
        );
    }
}
