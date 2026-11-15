//! Test doubles shared by the probe tests.
//!
//! Compiled only for tests: nothing in the shipped crate knows about fakes.

use std::collections::HashMap;

use crate::runner::{CommandOutput, CommandRunner};

/// A runner that replays recorded tool output.
///
/// Keys are `"program arg1 arg2"`. An unrecorded program behaves like a tool that
/// is not installed (the common case on a machine without that vendor).
#[derive(Debug, Default)]
pub struct FakeRunner {
    responses: HashMap<String, std::io::Result<CommandOutput>>,
}

impl FakeRunner {
    /// Record a successful invocation.
    pub fn with(mut self, key: &str, output: CommandOutput) -> Self {
        self.responses.insert(key.to_owned(), Ok(output));
        self
    }

    /// Record a tool that is installed but fails.
    pub fn failing(mut self, key: &str, status: i32, stderr: &str) -> Self {
        self.responses.insert(
            key.to_owned(),
            Ok(CommandOutput {
                status: Some(status),
                stdout: String::new(),
                stderr: format!("{stderr}\n"),
            }),
        );
        self
    }

    /// Record a tool that is not installed.
    pub fn missing(mut self, key: &str) -> Self {
        self.responses.insert(
            key.to_owned(),
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "simulated missing tool",
            )),
        );
        self
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<CommandOutput> {
        let key = format!("{program} {}", args.join(" "));
        match self.responses.get(key.trim()) {
            Some(Ok(output)) => Ok(output.clone()),
            Some(Err(err)) => Err(std::io::Error::new(err.kind(), "simulated")),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no recording for {key}"),
            )),
        }
    }
}

/// Convenience constructor for a successful invocation.
pub fn ok(stdout: &str) -> CommandOutput {
    CommandOutput {
        status: Some(0),
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}

/// Convenience constructor for a failing invocation.
pub fn failed(status: i32, stderr: &str) -> CommandOutput {
    CommandOutput {
        status: Some(status),
        stdout: String::new(),
        stderr: format!("{stderr}\n"),
    }
}
