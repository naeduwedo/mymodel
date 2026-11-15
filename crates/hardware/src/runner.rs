//! Command execution, behind a trait so probes can be tested without hardware.
//!
//! Every vendor probe is "run a tool, parse its output". Testing that by
//! invoking the real tool couples the test suite to whatever GPU the machine
//! happens to have; testing it through a recorded output does not. The trait is
//! the seam between those two worlds.

use std::process::Command;

/// Result of running a helper tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// Exit code, when the process ran at all.
    pub status: Option<i32>,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
}

impl CommandOutput {
    /// True when the tool ran and exited successfully.
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// First non-empty line of the tool's error output.
    ///
    /// Falls back to stdout: nvidia-smi topo -m prints "Failed to run topology
    /// matrix" on stdout on some Windows driver builds and leaves stderr empty.
    /// Reporting "no output" there would hide the only explanation the user has.
    pub fn stderr_line(&self) -> String {
        first_line(&self.stderr)
            .or_else(|| first_line(&self.stdout))
            .unwrap_or_else(|| "no output".to_owned())
    }

    /// First non-empty stdout line, for a user-facing reason.
    pub fn stdout_line(&self) -> String {
        first_line(&self.stdout).unwrap_or_else(|| "no output".to_owned())
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(strip_ansi)
}

/// Remove ANSI escape sequences from tool output.
///
/// `nvidia-smi` decorates its headers (underline, colour) even when its output is
/// not a terminal. Those bytes end up in a UI string and in log lines, where they
/// render as garbage. Stripping them here keeps every consumer honest instead of
/// asking each one to clean up.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();

    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }

        // CSI: ESC '[' ... final byte in 0x40..=0x7E
        if let Some('[') = chars.clone().next() {
            chars.next();
            for next in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&next) {
                    break;
                }
            }
        }
    }

    out.trim().to_owned()
}

/// Runs helper tools. Implemented for the real system and for tests.
pub trait CommandRunner {
    /// Run a program with arguments.
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<CommandOutput>;
}

/// Runs the real tools on this machine.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<CommandOutput> {
        let output = Command::new(program).args(args).output()?;
        Ok(CommandOutput {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_needs_a_zero_exit_code() {
        assert!(
            CommandOutput {
                status: Some(0),
                stdout: String::new(),
                stderr: String::new()
            }
            .success()
        );
        assert!(
            !CommandOutput {
                status: Some(1),
                stdout: String::new(),
                stderr: String::new()
            }
            .success()
        );
        assert!(
            !CommandOutput {
                status: None,
                stdout: String::new(),
                stderr: String::new()
            }
            .success()
        );
    }

    #[test]
    fn failure_text_falls_back_to_stdout() {
        // Observed on a real Windows driver build: the error goes to stdout.
        let output = CommandOutput {
            status: Some(255),
            stdout: "GPU0 GPU1\nFailed to run topology matrix\n".to_owned(),
            stderr: String::new(),
        };
        assert_eq!(output.stderr_line(), "GPU0 GPU1");
    }

    #[test]
    fn ansi_escapes_are_stripped_from_tool_output() {
        // nvidia-smi underlines its topology header even when piped.
        let raw = "\u{1b}[4mGPU0\tGPU1\u{1b}[0m CPU Affinity";
        assert_eq!(strip_ansi(raw), "GPU0\tGPU1 CPU Affinity");

        let colour = "\u{1b}[31mFailed to run topology matrix\u{1b}[0m";
        assert_eq!(strip_ansi(colour), "Failed to run topology matrix");

        // Text without escapes is untouched.
        assert_eq!(strip_ansi("plain text"), "plain text");
        // A dangling escape must not panic or truncate the rest.
        assert_eq!(strip_ansi("a\u{1b}"), "a");
    }

    #[test]
    fn failure_reasons_never_contain_escape_sequences() {
        let output = CommandOutput {
            status: Some(255),
            stdout: "\u{1b}[4mGPU0\tGPU1\u{1b}[0m\n".to_owned(),
            stderr: String::new(),
        };
        assert!(!output.stderr_line().contains('\u{1b}'));
    }

    #[test]
    fn reasons_skip_blank_lines() {
        let output = CommandOutput {
            status: Some(255),
            stdout: String::new(),
            stderr: "\n  \nFailed to run topology matrix\n".to_owned(),
        };
        assert_eq!(output.stderr_line(), "Failed to run topology matrix");
    }
}
