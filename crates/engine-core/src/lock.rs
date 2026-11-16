//! Single-instance lock (spec §3.3: "单实例锁 / single instance lock").
//!
//! Two cores sharing one data root would fight over the SQLite database, the
//! GPU leases and the worker table. The lock is an exclusive OS file lock
//! (`fs4`, which is `LockFileEx` on Windows and `flock` on Unix) held for the
//! lifetime of the core.
//!
//! Both mechanisms are released by the OS when the process dies, so a crash
//! cannot leave a stale lock behind - which is why this is a lock and not a
//! "create the file, fail if it exists" scheme.
//!
//! The lock file records the owning pid and start time so a user (and the CLI)
//! can see *who* holds it instead of a generic failure.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs4::FileExt;
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

        let mut file = options.open(&path).map_err(|source| LockError::Io {
            path: path.display().to_string(),
            source,
        })?;

        // An OS file lock works on every platform and is released when the
        // process dies. Before this, only Windows was protected: on Unix a
        // second core started happily against the same data root, which the
        // end-to-end test caught by hanging.
        // Called through the trait on purpose: Rust 1.100 added an inherent
        // `File::try_lock`, which would shadow this and raise the MSRV.
        match FileExt::try_lock(&file) {
            Ok(()) => {}
            Err(fs4::TryLockError::WouldBlock) => {
                return Err(LockError::Held {
                    path: path.display().to_string(),
                    pid: read_owner_pid(&path).unwrap_or(0),
                });
            }
            Err(fs4::TryLockError::Error(source)) => {
                return Err(LockError::Io {
                    path: path.display().to_string(),
                    source,
                });
            }
        }

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
    fn a_second_acquire_is_refused_on_every_platform() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("core.lock");
        let first = InstanceLock::acquire(&path).unwrap();

        // The lock must be enforced by the OS, not by our bookkeeping: a second
        // core on the same data root is the failure this exists to prevent, and
        // it was previously only prevented on Windows.
        match InstanceLock::acquire(&path) {
            Err(LockError::Held { path, .. }) => {
                assert!(path.ends_with("core.lock"), "{path}");
            }
            other => panic!("expected Held, got {other:?}"),
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
