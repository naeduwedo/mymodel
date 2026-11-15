//! Administrative queries used by health checks and `gguf doctor`.

use crate::DbError;

/// Summary of the database as reported over IPC and in the health check.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DatabaseStatus {
    /// File path, or `:memory:`.
    pub path: String,
    /// Applied schema version.
    pub schema_version: i64,
    /// `sqlite_version()` of the linked library.
    pub sqlite_version: String,
    /// Journal mode actually in use (`wal` outside tests).
    pub journal_mode: String,
    /// Size of the database file in bytes (0 for in-memory databases).
    pub file_bytes: i64,
}

impl crate::Database {
    /// Collect a status snapshot.
    pub async fn status(&self) -> Result<DatabaseStatus, DbError> {
        let path = self.path().display().to_string();
        self.call(move |conn| {
            let schema_version: i64 = conn.query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )?;
            let sqlite_version: String =
                conn.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
            let journal_mode: String =
                conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
            // `page_count * page_size` is the size of the main database; an
            // in-memory database reports its allocated pages too, so we zero
            // it explicitly for readers that expect a file size.
            let file_bytes: i64 = conn
                .query_row(
                    "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            let file_bytes = if path_is_memory(&path) { 0 } else { file_bytes };
            Ok(DatabaseStatus {
                path,
                schema_version,
                sqlite_version,
                journal_mode,
                file_bytes,
            })
        })
        .await
    }
}

fn path_is_memory(path: &str) -> bool {
    path == ":memory:"
}

#[cfg(test)]
mod tests {
    // `super::*` is not needed: the tests call methods on `crate::Database`,
    // which is already in scope through the module path.
    #[tokio::test]
    async fn status_reports_the_linked_sqlite_version() {
        let db = crate::Database::open_in_memory().unwrap();
        let status = db.status().await.unwrap();
        assert_eq!(status.schema_version, crate::latest_version());
        assert!(status.sqlite_version.starts_with('3'), "{status:?}");
        assert_eq!(status.path, ":memory:");
    }
}
