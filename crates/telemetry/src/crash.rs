//! Crash reporting (spec §13 Phase 6: crash logs).
//!
//! A desktop app that crashes silently is unshippable: the user cannot tell
//! support what happened, and the log they do have usually stops one line before
//! the failure. This module installs a panic hook that writes one self-contained
//! file, then runs the previous hook so console behaviour is unchanged.
//!
//! The hook is split in two on purpose:
//!
//! * [`install`] registers the hook and extracts the facts from `PanicHookInfo`;
//! * [`write_report`] takes plain data, so it can be tested without panicking.
//!
//! The hook must never panic itself (that aborts with no file) and must not
//! allocate a lock, so it uses plain `std::fs`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// File name prefix for crash reports.
pub const CRASH_PREFIX: &str = "crash-";

/// The facts a crash report records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashFacts {
    /// UTC epoch milliseconds.
    pub at_unix_ms: u64,
    /// `file:line:column`, or `<unknown>`.
    pub location: String,
    /// The panic message.
    pub message: String,
}

/// Install the hook, writing reports into `log_dir`.
///
/// Returns the directory the hook will use, so the caller can print it.
pub fn install(log_dir: &Path) -> PathBuf {
    let directory = log_dir.to_path_buf();
    let _ = std::fs::create_dir_all(&directory);

    let previous = std::panic::take_hook();
    let for_hook = directory.clone();

    std::panic::set_hook(Box::new(move |info| {
        let message = if let Some(text) = info.payload().downcast_ref::<&str>() {
            (*text).to_owned()
        } else if let Some(text) = info.payload().downcast_ref::<String>() {
            text.clone()
        } else {
            "<non-string panic payload>".to_owned()
        };

        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "<unknown>".to_owned());

        let facts = CrashFacts {
            at_unix_ms: unix_millis(),
            location,
            message,
        };

        // Best effort: a failure here must not mask the original panic.
        if let Some(path) = write_report(&for_hook, &facts) {
            eprintln!("crash report written to {}", path.display());
        }

        previous(info);
    }));

    directory
}

/// Write one crash report. Returns the path on success.
pub fn write_report(log_dir: &Path, facts: &CrashFacts) -> Option<PathBuf> {
    let path = log_dir.join(format!("{CRASH_PREFIX}{}.log", facts.at_unix_ms));

    let mut file = std::fs::File::create(&path).ok()?;
    let _ = writeln!(file, "GGUF AI Engine crash report");
    let _ = writeln!(file, "at        : {} (unix ms)", facts.at_unix_ms);
    let _ = writeln!(file, "version   : {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        file,
        "platform  : {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let _ = writeln!(file, "location  : {}", facts.location);
    let _ = writeln!(file, "message   : {}", facts.message);
    let _ = file.flush();

    Some(path)
}

/// Existing crash reports in a directory, newest first.
pub fn reports(log_dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(log_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.starts_with(CRASH_PREFIX))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();

    // Names embed a millisecond timestamp, so a reverse lexical sort is newest
    // first without parsing anything.
    found.sort_by(|a, b| b.cmp(a));
    found
}

fn unix_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> CrashFacts {
        CrashFacts {
            at_unix_ms: 1_700_000_000_000,
            location: "src/main.rs:42:9".to_owned(),
            message: "index out of bounds".to_owned(),
        }
    }

    #[test]
    fn a_report_records_version_platform_and_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_report(dir.path(), &facts()).expect("report written");

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("GGUF AI Engine crash report"));
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert!(text.contains(std::env::consts::OS));
        assert!(text.contains("index out of bounds"));
        assert!(text.contains("src/main.rs:42:9"));
    }

    #[test]
    fn the_file_name_carries_the_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_report(dir.path(), &facts()).unwrap();
        assert!(
            path.file_name().unwrap().to_string_lossy() == "crash-1700000000000.log",
            "{}",
            path.display()
        );
    }

    #[test]
    fn reports_are_listed_newest_first_and_filtered() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("crash-100.log"), b"a").unwrap();
        std::fs::write(dir.path().join("crash-200.log"), b"b").unwrap();
        std::fs::write(dir.path().join("core.log"), b"c").unwrap();

        let found = reports(dir.path());
        assert_eq!(found.len(), 2, "only crash-* files count");
        assert!(found[0].to_string_lossy().contains("crash-200"));
    }

    #[test]
    fn a_missing_directory_yields_no_reports_rather_than_panicking() {
        assert!(reports(Path::new("no-such-directory-xyz")).is_empty());
    }
}
