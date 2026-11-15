//! Windows named-pipe name rules and security descriptor.
//!
//! The pipe must be reachable by the user running the core (and by SYSTEM, so
//! services and the installer can query health) and by nobody else. The
//! descriptor is built from the current user's SID rather than a well-known
//! group, so on a shared machine another local user cannot poke the API.
//!
//! This module contains the workspace's only `unsafe` code; see
//! `docs/adr/ADR-010-unsafe-windows-pipe-acl.md` for why and for the review
//! rules that apply to changes here.

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PWSTR;

use crate::SecurityError;

/// DACL granting full access to SYSTEM and to one user SID.
///
/// `D:P` is a protected DACL (nothing inherited); there is one ACE for SYSTEM
/// and one for the user, and no ACE for Everyone or Authenticated Users.
pub(crate) fn sddl_for_user(sid: &str) -> String {
    format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})")
}

/// Rules for a named-pipe endpoint name (every platform).
///
/// Windows limits a pipe name to 256 characters, and a client may only connect
/// to a local pipe under `\\.\pipe\`. Both are checked before a name is used,
/// so a misconfigured data directory fails loudly at startup instead of
/// producing an unreachable endpoint.
pub(crate) fn validate_pipe_name(name: &str) -> Result<(), SecurityError> {
    const PREFIX: &str = "\\\\.\\pipe\\";
    if !name.starts_with(PREFIX) {
        return Err(SecurityError::Endpoint(format!(
            "named pipe must start with {PREFIX}, got {name}"
        )));
    }
    if name.len() > 256 {
        return Err(SecurityError::Endpoint(format!(
            "named pipe name is {} characters, the limit is 256",
            name.len()
        )));
    }
    if name.contains(['/', '\\']) && name[PREFIX.len()..].contains(['/', '\\']) {
        return Err(SecurityError::Endpoint(
            "named pipe names cannot contain path separators".into(),
        ));
    }
    Ok(())
}

/// Resolve the current process user's SID as a string.
pub(crate) fn current_user_sid() -> Result<String, SecurityError> {
    // SAFETY: this is the documented Win32 sequence for TOKEN_USER. The token
    // handle is closed on every path, the information buffer is owned by this
    // function, and the SID string is freed with LocalFree after conversion.
    unsafe {
        let mut token = Default::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|err| SecurityError::Endpoint(format!("OpenProcessToken: {err}")))?;

        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return Err(SecurityError::Endpoint(
                "GetTokenInformation returned no size".into(),
            ));
        }

        let mut buffer = vec![0u8; needed as usize];
        let result = GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        );
        let _ = CloseHandle(token);
        result.map_err(|err| SecurityError::Endpoint(format!("GetTokenInformation: {err}")))?;

        // SAFETY: GetTokenInformation wrote a TOKEN_USER into the buffer, which
        // outlives this reference.
        let token_user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid_string = PWSTR::null();
        ConvertSidToStringSidW(token_user.User.Sid, &mut sid_string)
            .map_err(|err| SecurityError::Endpoint(format!("ConvertSidToStringSidW: {err}")))?;
        let sid = sid_string.to_string().unwrap_or_default();
        let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            sid_string.0.cast(),
        )));

        if sid.is_empty() {
            return Err(SecurityError::Endpoint("current user SID is empty".into()));
        }
        Ok(sid)
    }
}

/// Build `SECURITY_ATTRIBUTES` for pipe creation.
///
/// The returned structure points at a descriptor allocated by `LocalAlloc`.
/// It is deliberately leaked for the process lifetime: every accepted
/// connection creates a new pipe instance that needs the same attributes, and
/// the allocation is a few hundred bytes for the life of the daemon.
pub(crate) fn current_user_security_attributes()
-> Result<windows::Win32::Security::SECURITY_ATTRIBUTES, SecurityError> {
    let sddl = current_user_sddl()?;
    let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: `wide` is NUL terminated and outlives the call. The descriptor is
    // owned by LocalAlloc and intentionally outlives this function.
    unsafe {
        let mut descriptor = windows::Win32::Security::PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            windows::core::PCWSTR(wide.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|err| {
            SecurityError::Endpoint(format!("ConvertStringSecurityDescriptor: {err}"))
        })?;

        // `..Default::default()` gives bInheritHandle = 0 (not inherited) and
        // keeps this compiling across `windows` releases that change the BOOL
        // representation.
        Ok(windows::Win32::Security::SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<windows::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            ..Default::default()
        })
    }
}

/// SDDL string describing the DACL for this process's user.
///
/// Kept as a `String` rather than `SECURITY_ATTRIBUTES` so it can be asserted in
/// a test and logged (it contains a SID, which is not a secret).
pub fn current_user_sddl() -> Result<String, SecurityError> {
    Ok(sddl_for_user(&current_user_sid()?))
}

/// Validate an SDDL string by converting it with the OS parser.
///
/// Used by the tests to prove the DACL we generate is one the OS accepts; the
/// runtime path does not need it because a bad DACL makes pipe creation fail at
/// startup, which is already loud.
#[cfg(test)]
pub(crate) fn validate_sddl(sddl: &str) -> Result<(), SecurityError> {
    use windows::Win32::Security::PSECURITY_DESCRIPTOR;
    let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: `wide` is NUL terminated and outlives the call. The descriptor is
    // owned by LocalAlloc, so it is freed before returning.
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            windows::core::PCWSTR(wide.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|err| {
            SecurityError::Endpoint(format!("ConvertStringSecurityDescriptor: {err}"))
        })?;
        let _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            descriptor.0,
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sddl_grants_only_system_and_the_user() {
        let sddl = sddl_for_user("S-1-5-21-1-2-3-1001");
        assert!(sddl.starts_with("D:P"));
        assert!(sddl.contains(";;;SY)"));
        assert!(sddl.contains(";;;S-1-5-21-1-2-3-1001)"));
        assert!(!sddl.contains(";;;WD"), "Everyone must not be granted");
        assert!(
            !sddl.contains(";;;AU"),
            "Authenticated Users must not be granted"
        );
    }

    #[test]
    fn current_user_sid_is_resolved() {
        let sid = current_user_sid().expect("current user SID");
        assert!(sid.starts_with("S-1-"), "{sid}");
    }

    #[test]
    fn the_generated_dacl_is_accepted_by_the_os_parser() {
        // Proves the SDDL we hand to the pipe is actually valid, not merely
        // well-formed-looking.
        let sddl = current_user_sddl().expect("sddl");
        validate_sddl(&sddl).expect("OS accepts the DACL");
    }

    #[test]
    fn pipe_names_are_validated() {
        assert!(validate_pipe_name(r"\\.\pipe\gguf-ai-engine-abc").is_ok());
        assert!(validate_pipe_name("gguf-ai-engine-abc").is_err());
        assert!(validate_pipe_name(&format!(r"\\.\pipe\{}", "x".repeat(300))).is_err());
        assert!(validate_pipe_name(r"\\.\pipe\bad/name").is_err());
    }
}
