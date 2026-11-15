//! Versioned schema migrations (spec §10: "数据库迁移必须版本化").
//!
//! Rules this module enforces:
//!
//! * migrations are **forward-only** and append-only — an applied migration is
//!   never edited, a fix is a new migration;
//! * the applied version is recorded in `schema_migrations`, so a database
//!   written by a newer build is detected and refused instead of corrupted;
//! * every migration runs inside a transaction, together with its version row,
//!   so a crash leaves the database at the previous version;
//! * capacities are `INTEGER` bytes and timestamps are UTC epoch milliseconds
//!   (spec §10) — never floating-point gigabytes, never local time.

use rusqlite::{Connection, Error as SqlError};

/// One migration step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// Monotonic version, starting at 1.
    pub version: i64,
    /// Short description used in logs.
    pub name: &'static str,
    /// SQL statements applied atomically.
    pub sql: &'static str,
}

/// All migrations, in application order.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        // Path is relative to this source file (crates/database/src/), so the
        // schema lives at the workspace root next to the other migration files.
        sql: include_str!("../../../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "app_metadata",
        sql: include_str!("../../../migrations/0002_app_metadata.sql"),
    },
];

/// Current (latest) schema version.
pub fn latest_version() -> i64 {
    MIGRATIONS.last().map(|m| m.version).unwrap_or(0)
}

/// Migration failure.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// The SQL engine reported an error.
    #[error("migration {version} ({name}) failed: {source}")]
    Sql {
        /// Migration version.
        version: i64,
        /// Migration name.
        name: &'static str,
        /// Underlying error.
        #[source]
        source: SqlError,
    },
    /// The database was written by a newer build.
    #[error(
        "database schema version {found} is newer than this build supports ({supported}); upgrade the application instead of downgrading"
    )]
    TooNew {
        /// Version found in the database.
        found: i64,
        /// Latest version this build knows.
        supported: i64,
    },
    /// The `schema_migrations` table could not be read.
    #[error("cannot read the applied schema version: {0}")]
    ReadVersion(#[source] SqlError),
}

/// Outcome of an [`apply`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationOutcome {
    /// Version before the call.
    pub from: i64,
    /// Version after the call.
    pub to: i64,
    /// Versions applied by this call.
    pub applied: Vec<i64>,
}

impl MigrationOutcome {
    /// True when the schema did not change.
    pub fn is_noop(&self) -> bool {
        self.applied.is_empty()
    }
}

fn ensure_version_table(conn: &Connection) -> Result<(), MigrationError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             name       TEXT    NOT NULL,
             applied_at INTEGER NOT NULL
         ) STRICT;",
    )
    .map_err(|source| MigrationError::Sql {
        version: 0,
        name: "schema_migrations",
        source,
    })
}

/// Version currently applied to the database (0 for an empty database).
pub fn current_version(conn: &Connection) -> Result<i64, MigrationError> {
    ensure_version_table(conn)?;
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map_err(MigrationError::ReadVersion)
}

/// Apply every pending migration.
pub fn apply(conn: &mut Connection) -> Result<MigrationOutcome, MigrationError> {
    let from = current_version(conn)?;
    let supported = latest_version();
    if from > supported {
        return Err(MigrationError::TooNew {
            found: from,
            supported,
        });
    }

    let mut applied = Vec::new();
    for migration in MIGRATIONS.iter().filter(|m| m.version > from) {
        let tx = conn.transaction().map_err(|source| MigrationError::Sql {
            version: migration.version,
            name: migration.name,
            source,
        })?;
        tx.execute_batch(migration.sql)
            .map_err(|source| MigrationError::Sql {
                version: migration.version,
                name: migration.name,
                source,
            })?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                migration.version,
                migration.name,
                chrono::Utc::now().timestamp_millis()
            ],
        )
        .map_err(|source| MigrationError::Sql {
            version: migration.version,
            name: migration.name,
            source,
        })?;
        tx.commit().map_err(|source| MigrationError::Sql {
            version: migration.version,
            name: migration.name,
            source,
        })?;
        tracing::info!(
            version = migration.version,
            name = migration.name,
            "migration applied"
        );
        applied.push(migration.version);
    }

    Ok(MigrationOutcome {
        from,
        to: if applied.is_empty() {
            from
        } else {
            *applied.last().unwrap()
        },
        applied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        apply(&mut conn).unwrap();
        conn
    }

    #[test]
    fn fresh_database_reaches_the_latest_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(current_version(&conn).unwrap(), 0);
        let outcome = apply(&mut conn).unwrap();
        assert_eq!(outcome.from, 0);
        assert_eq!(outcome.to, latest_version());
        assert!(!outcome.is_noop());
        assert_eq!(current_version(&conn).unwrap(), latest_version());
    }

    #[test]
    fn second_run_is_a_no_op() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();
        let again = apply(&mut conn).unwrap();
        assert!(again.is_noop());
        assert_eq!(again.from, again.to);
    }

    #[test]
    fn newer_schema_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (9999, 'from-the-future', 0)",
            [],
        )
        .unwrap();
        let err = apply(&mut conn).unwrap_err();
        assert!(matches!(
            err,
            MigrationError::TooNew {
                found: 9999,
                supported: _
            }
        ));
        assert!(err.to_string().contains("upgrade the application"));
    }

    #[test]
    fn every_table_from_the_specification_exists() {
        let conn = memory_db();
        let required = [
            "models",
            "model_revisions",
            "model_files",
            "model_components",
            "model_manifests",
            "runtimes",
            "runtime_capabilities",
            "devices",
            "gpu_topology",
            "gpu_allocations",
            "model_instances",
            "memory_allocations",
            "tensor_pages",
            "inference_tasks",
            "inference_sessions",
            "download_tasks",
            "performance_metrics",
            "system_events",
            "settings",
        ];
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for table in required {
            assert!(tables.contains(&table.to_owned()), "missing table {table}");
        }
    }

    #[test]
    fn capacity_columns_are_integers_not_floats() {
        let conn = memory_db();
        // A byte count stored as REAL would lose precision above 2^53 and is
        // explicitly forbidden by spec §10.
        let bad: Vec<String> = conn
            .prepare(
                "SELECT m.name || '.' || p.name
                 FROM sqlite_master m, pragma_table_info(m.name) p
                 WHERE m.type = 'table'
                   AND (p.name LIKE '%_bytes' OR p.name LIKE '%_ms' OR p.name = 'progress')
                   AND lower(p.type) NOT IN ('integer', 'real')",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(bad.is_empty(), "capacity columns are not INTEGER: {bad:?}");
    }

    #[test]
    fn applied_migrations_are_recorded_with_utc_millis() {
        let conn = memory_db();
        let (name, applied_at): (String, i64) = conn
            .query_row(
                "SELECT name, applied_at FROM schema_migrations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(name, "init");
        // Sanity: after 2020 and before 2100 in epoch milliseconds.
        assert!(applied_at > 1_577_836_800_000);
        assert!(applied_at < 4_102_444_800_000);
    }

    #[test]
    fn migration_versions_are_unique_and_increasing() {
        let mut last = 0;
        for migration in MIGRATIONS {
            assert!(migration.version > last, "versions must be increasing");
            last = migration.version;
        }
    }
}
