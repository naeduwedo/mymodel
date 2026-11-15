//! `gguf-worker` — isolated inference runtime worker (spec §3.1, ADR-007).
//!
//! # Status: skeleton (Task 6 for llama.cpp, Task 8 for multimodal)
//!
//! The worker exists as a separate process from day one so that the invariant
//! "a model or driver crash must not take down the core" is structural rather
//! than a later refactor. It currently refuses to start any runtime: there is
//! no adapter yet, and pretending otherwise would violate spec rule 4.

use std::process::ExitCode;

/// Exit code used while no runtime adapter is available.
const EXIT_UNSUPPORTED: u8 = 4;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    eprintln!(
        "gguf-worker {}: no runtime adapter is installed in this build.",
        env!("CARGO_PKG_VERSION")
    );
    if !args.is_empty() {
        eprintln!("requested runtime: {}", args.join(" "));
    }
    eprintln!("runtime adapters arrive with Task 6 (llama.cpp); refusing to start.");
    ExitCode::from(EXIT_UNSUPPORTED)
}
