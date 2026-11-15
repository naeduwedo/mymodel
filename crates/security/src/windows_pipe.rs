//! Creating named pipes with a restrictive security descriptor.
//!
//! `tokio::net::windows::named_pipe::ServerOptions::create` passes a null
//! `SECURITY_ATTRIBUTES`, which means the pipe inherits the process default
//! DACL — on a typical desktop that grants access to the local Users group, not
//! just to the owner. The core must not be reachable by every local account, so
//! pipe instances are created through
//! `create_with_security_attributes_raw` with the DACL built from the current
//! user's SID (see `windows_acl::current_user_sddl`).
//!
//! This is the second (and last) `unsafe` site in the workspace; see
//! `docs/adr/ADR-010-unsafe-windows-pipe-acl.md`.

use std::ffi::OsStr;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

use crate::SecurityError;
use crate::windows_acl;

/// A pipe instance that has not accepted a client yet.
#[derive(Debug)]
pub struct PipeInstance {
    server: NamedPipeServer,
}

impl PipeInstance {
    /// Wait for a client to connect.
    pub async fn connect(self) -> std::io::Result<NamedPipeServer> {
        self.server.connect().await?;
        Ok(self.server)
    }

    /// The underlying tokio server.
    pub fn into_inner(self) -> NamedPipeServer {
        self.server
    }

    /// Open a client connection to a pipe created by [`create`].
    ///
    /// Used by tests to prove the DACL does not lock the creating user out.
    #[cfg(test)]
    pub fn client(name: &str) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
        tokio::net::windows::named_pipe::ClientOptions::new().open(name)
    }
}

/// Create a pipe instance with the user-only DACL applied.
///
/// The descriptor is intentionally leaked for the process lifetime: each new
/// connection needs the same attributes, and the allocation is a few hundred
/// bytes for the entire life of the daemon.
pub fn create(name: impl AsRef<OsStr>) -> Result<PipeInstance, SecurityError> {
    let attributes = windows_acl::current_user_security_attributes()?;
    let mut attributes = attributes;

    // SAFETY: `attributes` is a valid, initialised SECURITY_ATTRIBUTES whose
    // lpSecurityDescriptor is owned by this process for its lifetime. tokio
    // copies what it needs while creating the pipe and does not retain the
    // pointer, so the reference cannot dangle before it is used.
    let server = unsafe {
        ServerOptions::new().create_with_security_attributes_raw(
            name,
            (&mut attributes as *mut windows::Win32::Security::SECURITY_ATTRIBUTES).cast(),
        )
    }
    .map_err(|err| SecurityError::Endpoint(format!("creating the named pipe: {err}")))?;

    Ok(PipeInstance { server })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creates_a_pipe_with_the_user_dacl() {
        let name = format!(
            "{}gguf-ai-engine-test-{}",
            gguf_engine_protocol_prefix(),
            std::process::id()
        );
        let instance = create(&name).expect("pipe created");
        // The instance exists even before a client connects: dropping it here
        // must simply close the pipe without panicking.
        drop(instance);

        // A second instance on the same name is allowed (the core keeps a spare
        // listening instance), and a client can connect to it.
        let again = create(&name).expect("second instance");
        let client = PipeInstance::client(&name);
        assert!(client.is_ok(), "a client must be able to open the pipe");
        drop(again);
    }

    fn gguf_engine_protocol_prefix() -> String {
        gguf_engine_protocol::PIPE_PREFIX.to_owned()
    }
}
