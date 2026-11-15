//! SQLite storage for GGUF AI Engine (spec §10).
//!
//! # Concurrency model
//!
//! SQLite is synchronous. Rather than pull an async driver into the core,
//! [`Database`] keeps one `rusqlite` connection behind a mutex and runs every
//! statement on `spawn_blocking`, so a slow write can never stall the async
//! scheduler. The pragmas below make that safe under concurrent workers:
//!
//! * `journal_mode = WAL` — readers never block the writer;
//! * `synchronous = NORMAL` — durable across application crashes (not power
//!   loss); the cost of `FULL` is not justified for a local registry;
//! * `busy_timeout` — writers wait instead of failing with SQLITE_BUSY;
//! * `foreign_keys = ON` — the schema declares relationships, so they must be
//!   enforced (SQLite defaults to off, which silently allows orphans).

#![warn(missing_docs)]

pub mod admin;
mod migrations;
pub mod models;
pub mod registry;

pub use registry::{FileRecord, FileState, RevisionRecord};

pub use migrations::{
    MIGRATIONS, Migration, MigrationError, MigrationOutcome, apply, current_version, latest_version,
};

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::Connection;
use thiserror::Error;

/// Storage failures.
#[derive(Debug, Error)]
pub enum DbError {
    /// Opening or configuring the database failed.
    #[error("opening the database at {path}: {source}")]
    Open {
        /// Database file path.
        path: String,
        /// Underlying error.
        #[source]
        source: rusqlite::Error,
    },

    /// A statement failed.
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),

    /// A migration failed.
    #[error(transparent)]
    Migration(#[from] MigrationError),

    /// The caller asked for a record that does not exist.
    #[error("{0} not found")]
    NotFound(String),

    /// A blocking task panicked or was cancelled.
    #[error("database task failed: {0}")]
    Task(String),

    /// JSON in a column could not be decoded.
    #[error("decoding stored JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// An open, migrated database.
#[derive(Debug, Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
    in_memory: bool,
}

impl Database {
    /// Open (creating if needed) and migrate a database file.
    ///
    /// Migrating an existing database copies it first (see
    /// [`Self::open_with_backup`]); this convenience form discards the backup
    /// path. Callers that want to tell the user where the backup went should use
    /// the explicit form.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        Self::open_with_backup(path).map(|(database, _)| database)
    }

    /// Open and migrate, returning the path of the pre-migration backup when one
    /// was made.
    ///
    /// Spec §10 requires a backup strategy for migrations. The rule here:
    ///
    /// * a **fresh** database is not backed up (there is nothing to lose);
    /// * an **up-to-date** database is not backed up (nothing will change);
    /// * a database that will actually be migrated is copied to
    ///   `<name>.backup-v<from>` before the first migration runs.
    ///
    /// A failed backup aborts the open: running a migration on unbacked-up user
    /// data is exactly what the requirement exists to prevent.
    pub fn open_with_backup(path: impl AsRef<Path>) -> Result<(Self, Option<PathBuf>), DbError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| DbError::Open {
                path: parent.display().to_string(),
                source: rusqlite::Error::ToSqlConversionFailure(Box::new(source)),
            })?;
        }

        let mut conn = Connection::open(&path).map_err(|source| DbError::Open {
            path: path.display().to_string(),
            source,
        })?;
        configure(&conn, &path)?;

        // Decide whether a backup is needed before touching anything.
        let existing_version = current_version(&conn)?;
        let backup = if existing_version > 0 && existing_version < latest_version() {
            let backup_path = path.with_extension(format!("backup-v{existing_version}"));
            std::fs::copy(&path, &backup_path).map_err(|source| DbError::Open {
                path: backup_path.display().to_string(),
                source: rusqlite::Error::ToSqlConversionFailure(Box::new(source)),
            })?;
            tracing::info!(
                from = existing_version,
                to = latest_version(),
                backup = %backup_path.display(),
                "backed up the database before migrating"
            );
            Some(backup_path)
        } else {
            None
        };

        let outcome = apply(&mut conn)?;
        tracing::info!(
            from = outcome.from,
            to = outcome.to,
            applied = ?outcome.applied,
            path = %path.display(),
            "database ready"
        );
        Ok((
            Self {
                conn: Arc::new(Mutex::new(conn)),
                path,
                in_memory: false,
            },
            backup,
        ))
    }

    /// Open an in-memory database (tests).
    pub fn open_in_memory() -> Result<Self, DbError> {
        let mut conn = Connection::open_in_memory().map_err(|source| DbError::Open {
            path: ":memory:".to_owned(),
            source,
        })?;
        configure(&conn, Path::new(":memory:"))?;
        apply(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: PathBuf::from(":memory:"),
            in_memory: true,
        })
    }

    /// File path (or `:memory:`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// True for an in-memory database.
    pub fn is_in_memory(&self) -> bool {
        self.in_memory
    }

    /// Run a closure with the connection on the blocking pool.
    pub async fn call<T, F>(&self, f: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, DbError> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock();
            f(&mut guard)
        })
        .await
        .map_err(|err| DbError::Task(err.to_string()))?
    }

    /// Run a closure without leaving the current thread.
    ///
    /// Only for short, non-async call sites (startup checks, tests). Async code
    /// must use [`Database::call`] so it does not block the runtime.
    pub fn call_blocking<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        let mut guard = self.conn.lock();
        f(&mut guard)
    }

    /// Schema version currently applied.
    pub fn schema_version(&self) -> Result<i64, DbError> {
        self.call_blocking(|conn| Ok(current_version(conn)?))
    }
}

fn configure(conn: &Connection, path: &Path) -> Result<(), DbError> {
    let in_memory = path.as_os_str() == ":memory:";
    if !in_memory {
        // WAL keeps readers from blocking the writer, which matters once the
        // API and a worker both touch the registry. In-memory databases have
        // no journal file, so the pragma is simply skipped there.
        let mode: String = conn
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(DbError::Sql)?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(DbError::Sql(rusqlite::Error::InvalidQuery));
        }
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_migrates_a_file_database() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("database").join("engine.db");
        let db = Database::open(&path).unwrap();
        assert!(!db.is_in_memory());
        assert_eq!(db.schema_version().unwrap(), latest_version());
        assert!(path.exists());
    }

    #[test]
    fn reopening_keeps_the_schema_version() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("engine.db");
        {
            let db = Database::open(&path).unwrap();
            db.call_blocking(|conn| {
                conn.execute(
                    "INSERT INTO settings (key, value, updated_at) VALUES ('theme', 'dark', 1)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        }
        let db = Database::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), latest_version());
        let value: String = db
            .call_blocking(|conn| {
                Ok(conn.query_row("SELECT value FROM settings", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(value, "dark");
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let db = Database::open_in_memory().unwrap();
        let err = db
            .call_blocking(|conn| {
                conn.execute(
                    "INSERT INTO model_revisions (revision_id, model_id, revision, resolved_at)
                     VALUES ('r1', 'missing-model', 'main', 0)",
                    [],
                )?;
                Ok(())
            })
            .unwrap_err();
        assert!(err.to_string().contains("FOREIGN KEY"), "{err}");
    }

    #[tokio::test]
    async fn call_runs_on_the_blocking_pool() {
        let db = Database::open_in_memory().unwrap();
        let version = db.call(|conn| Ok(current_version(conn)?)).await.unwrap();
        assert_eq!(version, latest_version());
    }

    #[tokio::test]
    async fn concurrent_callers_do_not_deadlock() {
        let db = Database::open_in_memory().unwrap();
        let mut handles = Vec::new();
        for i in 0..16 {
            let db = db.clone();
            handles.push(tokio::spawn(async move {
                db.call(move |conn| {
                    conn.execute(
                        "INSERT INTO system_events (at, level, category, message)
                         VALUES (?1, 'info', 'test', 'concurrent')",
                        [i],
                    )?;
                    Ok(())
                })
                .await
            }));
        }
        for handle in handles {
            handle.await.unwrap().unwrap();
        }
        let count: i64 = db
            .call(
                |conn| Ok(conn.query_row("SELECT COUNT(*) FROM system_events", [], |r| r.get(0))?),
            )
            .await
            .unwrap();
        assert_eq!(count, 16);
    }
}
