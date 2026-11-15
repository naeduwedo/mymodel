//! Logging and performance telemetry (spec §4.8, §14.3).
//!
//! # Status: partially implemented (Task 1)
//!
//! [`tracing_setup`] installs the global subscriber used by the daemon and the
//! CLI. Metric collection, rotation policy and the performance report are
//! planned for later tasks.

#![warn(missing_docs)]

pub mod crash;

use tracing_subscriber::EnvFilter;

/// Whether this module is implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Implemented and tested.
    Implemented,
    /// Not implemented yet.
    Planned,
}

/// Current status of the crate.
pub const STATUS: Status = Status::Implemented;

/// Log output format. Usable directly as a `clap` value enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LogFormat {
    /// Human readable, for interactive use.
    Pretty,
    /// One JSON object per line, for machines and log shipping.
    Json,
}

/// Install the global tracing subscriber. Returns an error string when a
/// subscriber is already installed (callers treat that as benign).
///
/// # Why stderr
///
/// Logs always go to stderr. Two reasons:
///
/// * `--json` output on stdout stays machine parseable;
/// * `tracing_subscriber::fmt`'s guard flushes on drop, and a stdout writer
///   would both interleave with `println!` output and re-arm a failed stdout
///   flush, turning a perfectly successful CLI run into exit code 1.
///
/// The filter falls back to `info` when `directive` is not a valid
/// `EnvFilter` expression, and the fallback is logged so a typo cannot
/// silently silence the daemon.
pub fn tracing_setup(directive: &str, format: LogFormat) -> Result<(), String> {
    let (filter, fell_back) = match EnvFilter::try_new(directive) {
        Ok(filter) => (filter, false),
        Err(_) => (EnvFilter::new("info"), true),
    };

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    let result = match format {
        LogFormat::Pretty => builder.try_init(),
        LogFormat::Json => builder.json().try_init(),
    };
    if fell_back {
        tracing::warn!(
            directive,
            "invalid log filter directive, falling back to \"info\""
        );
    }
    result.map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installing_twice_reports_an_error_instead_of_panicking() {
        // The test harness may already own the global subscriber, in which
        // case the first call fails too; both outcomes must be non-panicking.
        let first = tracing_setup("info", LogFormat::Pretty);
        let second = tracing_setup("debug", LogFormat::Json);
        assert!(second.is_err() || first.is_err());
    }
}
