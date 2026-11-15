//! `gguf version` — version and build information.

use anyhow::Result;
use serde::Serialize;

use crate::output;

#[derive(Debug, Serialize)]
struct VersionInfo {
    cli: &'static str,
    core: &'static str,
    protocol: u32,
    os: &'static str,
    arch: &'static str,
    profile: &'static str,
}

/// Target triple facts about this build.
fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// Print version information.
pub fn version(json: bool) -> Result<()> {
    let info = VersionInfo {
        cli: env!("CARGO_PKG_VERSION"),
        core: gguf_engine_core::CORE_VERSION,
        protocol: gguf_engine_protocol::PROTOCOL_VERSION,
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        profile: build_profile(),
    };
    output::emit(json, &info, |info| {
        println!("gguf-ai-engine {}", info.cli);
        println!("  core     : {}", info.core);
        println!("  protocol : v{}", info.protocol);
        println!("  platform : {} / {}", info.os, info.arch);
        println!("  profile  : {}", info.profile);
    });
    Ok(())
}
