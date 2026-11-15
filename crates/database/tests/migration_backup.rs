//! Migration backup tests (spec §10: backup strategy and upgrade tests).
//!
//! These build a database at an **older** schema version and open it with the
//! current build, which is exactly what a user does when they upgrade the app.

use gguf_engine_database::{Database, MIGRATIONS, latest_version};
use rusqlite::Connection;

/// Create a database at version 1 only, the way an older release would have.
fn database_at_v1(path: &std::path::Path) {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();

    let first = &MIGRATIONS[0];
    conn.execute_batch(first.sql).unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL
         ) STRICT;",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO schema_migrations (version, name, applied_at) VALUES (1, 'init', 1)",
        [],
    )
    .unwrap();
}

#[test]
fn a_fresh_database_is_not_backed_up() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("engine.db");

    let (database, backup) = Database::open_with_backup(&path).unwrap();
    assert!(backup.is_none(), "nothing to lose in a new database");
    assert_eq!(database.schema_version().unwrap(), latest_version());
}

#[test]
fn an_up_to_date_database_is_not_backed_up() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("engine.db");

    drop(Database::open_with_backup(&path).unwrap());
    let (_database, backup) = Database::open_with_backup(&path).unwrap();
    assert!(backup.is_none(), "an up-to-date schema changes nothing");
}

#[test]
fn an_older_database_is_backed_up_then_upgraded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("engine.db");
    database_at_v1(&path);

    let (database, backup) = Database::open_with_backup(&path).unwrap();
    let backup = backup.expect("a migration must be backed up");

    assert!(backup.is_file(), "the backup file exists");
    assert!(
        backup
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("backup-v1"),
        "the name records the version it came from: {}",
        backup.display()
    );
    assert_eq!(database.schema_version().unwrap(), latest_version());
    assert!(latest_version() > 1, "the upgrade path must be exercised");
}

#[test]
fn the_backup_still_contains_the_previous_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("engine.db");
    database_at_v1(&path);

    // A user row written by the old release.
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('k', 'v', 1)",
            [],
        )
        .unwrap();
    }

    let (_database, backup) = Database::open_with_backup(&path).unwrap();
    let backup = backup.expect("backup");

    // The backup must still be a readable v1 database with the user's row.
    let conn = Connection::open(&backup).unwrap();
    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(version, 1, "the backup is the pre-upgrade schema");
    let value: String = conn
        .query_row("SELECT value FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(value, "v");
}

#[test]
fn a_database_from_a_newer_build_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("engine.db");

    {
        let database = Database::open(&path).unwrap();
        database
            .call_blocking(|conn| {
                conn.execute(
                    "INSERT INTO schema_migrations (version, name, applied_at) VALUES (9999, 'future', 0)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
    }

    let err = Database::open_with_backup(&path).unwrap_err();
    assert!(
        err.to_string().contains("newer than this build"),
        "a downgrade must be refused, not attempted: {err}"
    );
}

#[test]
fn the_upgrade_adds_the_app_metadata_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("engine.db");
    database_at_v1(&path);

    let (database, _) = Database::open_with_backup(&path).unwrap();
    let origin: String = database
        .call_blocking(|conn| {
            Ok(conn.query_row(
                "SELECT value FROM app_metadata WHERE key = 'schema_origin'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(origin, "0002_app_metadata");
}
