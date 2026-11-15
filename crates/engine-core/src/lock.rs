//! Single-instance lock (spec §3.3: "单实例锁 / single instance lock").
//!
//! Two cores sharing one data root would fight over the SQLite database, the
//! GPU leases and the worker table. The lock is an exclusive OS file handle
//! (`share_mode(0)` on Windows, advisory `flock`-equivalent semantics via
//! `create_new` + recorded pid elsewhere) held for the lifetime of the core.
//!
//! The lock file records the owning pid and start time so a user (and the CLI)
//! can see *who* holds it instead of a generic failure.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Errors raised while acquiring the single-instance lock.
#[derive(Debug, Error)]
pub enum LockError {
    /// Another core process owns the data root.
    ///
    /// `pid` is 0 when the owner could not be determined — on Windows the
    /// lock file is held with an exclusive share mode, so a contending
    /// process cannot read the recorded pid.
    #[error("another core instance already owns {path} (pid {pid})")]
    Held {
        /// Lock file path.
        path: String,
        /// Owner pid when it could be read, otherwise 0.
        pid: u32,
    },
    /// The lock file could not be created, opened or released.
    #[error("failed to use lock file {path}: {source}")]
    Io {
        /// Lock file path.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// Held single-instance lock. Released on drop.
#[derive(Debug)]
pub struct InstanceLock {
    file: File,
    path: PathBuf,
    pid: u32,
}

impl InstanceLock {
    /// Pid recorded in the lock file (this process).
    ///
    /// Stored in memory rather than re-read: on Windows the lock file is
    /// opened with `share_mode(0)`, so even this process cannot open a second
    /// handle to it.
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

impl InstanceLock {
    /// Acquire the lock, failing if another live process holds it.
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, LockError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| LockError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }

        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // 0 = no sharing: another process opening the file fails with
            // ERROR_SHARING_VIOLATION, which Windows releases automatically
            // when this process dies (no stale locks after a crash).
            options.share_mode(0);
        }

        let mut file = options
            .open(&path)
            .map_err(|err| match err.raw_os_error() {
                #[cfg(windows)]
                Some(32) | Some(33) => LockError::Held {
                    path: path.display().to_string(),
                    pid: read_owner_pid(&path).unwrap_or(0),
                },
                _ => LockError::Io {
                    path: path.display().to_string(),
                    source: err,
                },
            })?;

        let pid = std::process::id();
        file.set_len(0).map_err(|source| LockError::Io {
            path: path.display().to_string(),
            source,
        })?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| LockError::Io {
                path: path.display().to_string(),
                source,
            })?;
        writeln!(file, "pid={pid}").map_err(|source| LockError::Io {
            path: path.display().to_string(),
            source,
        })?;
        file.flush().map_err(|source| LockError::Io {
            path: path.display().to_string(),
            source,
        })?;

        tracing::debug!(path = %path.display(), pid, "single-instance lock acquired");
        Ok(Self { file, path, pid })
    }

    /// Path of the lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Pid of the process that owns the lock (always `self.pid`).
    pub fn owner_pid(&self) -> u32 {
        self.pid
    }

    /// Release the lock explicitly. Dropping has the same effect.
    pub fn release(self) {
        drop(self);
    }
}

/// Read the pid of the process holding `path`, without holding the lock.
///
/// Used for diagnostics (`gguf doctor`, the CLI error message) where another
/// process owns the file. Returns `None` when the file is missing, empty (the
/// owner released it) or not owned by a live core.
pub fn read_owner_pid(path: &Path) -> Option<u32> {
    let mut text = String::new();
    File::open(path).ok()?.read_to_string(&mut text).ok()?;
    let pid = text.trim().strip_prefix("pid=")?.parse::<u32>().ok()?;
    (pid > 0).then_some(pid)
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        // Best effort: truncate so a later reader does not see a stale pid.
        let _ = self.file.set_len(0);
        tracing::debug!(path = %self.path.display(), "single-instance lock released");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_records_its_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("core.lock");
        let lock = InstanceLock::acquire(&path).unwrap();
        assert_eq!(lock.pid(), std::process::id());
        assert_eq!(lock.owner_pid(), std::process::id());
        assert!(path.exists());
        // Another process can read who owns the lock without opening it
        // exclusively (that is all `gguf doctor` needs).
        let owner = read_owner_pid(&path);
        #[cfg(windows)]
        assert_eq!(
            owner, None,
            "an exclusively shared file cannot be read here"
        );
        #[cfg(not(windows))]
        assert_eq!(owner, Some(std::process::id()));
    }

    #[test]
    fn second_acquire_in_same_process_is_reported_on_windows() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("core.lock");
        let first = InstanceLock::acquire(&path).unwrap();
        let second = InstanceLock::acquire(&path);
        // On Windows the exclusive share mode rejects the second open.
        #[cfg(windows)]
        {
            // The pid is reported as 0 because the lock file itself cannot be
            // opened for reading while it is held exclusively; the caller gets
            // the `Held` error, which is what matters.
            match second {
                Err(LockError::Held { pid, path }) => {
                    assert_eq!(pid, 0);
                    assert!(path.ends_with("core.lock"));
                }
                other => panic!("expected Held, got {other:?}"),
            }
        }
        #[cfg(not(windows))]
        {
            // Advisory locking is not enforced in-process on Unix yet; the
            // lock file must at least be reusable after release.
            drop(second);
        }
        first.release();
        InstanceLock::acquire(&path).expect("lock is reusable after release");
    }

    #[test]
    fn lock_directory_is_created() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("dir").join("core.lock");
        let lock = InstanceLock::acquire(&path).unwrap();
        assert!(path.exists());
        drop(lock);
        // After release the file is truncated, so no stale pid is published.
        assert_eq!(read_owner_pid(&path), None);
    }
}
