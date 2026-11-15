//! Registry persistence for models, revisions and files (spec §7.2, §10).
//!
//! The registry stores **facts that were observed**: a file row is written only
//! after the bytes exist and their hash matched. A `ready` state therefore means
//! "verified", never "we intended to download this".

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{Database, DbError};

/// State of a model file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    /// Known from the repository listing, not downloaded.
    Pending,
    /// Partially transferred.
    Downloading,
    /// Present, hash being computed.
    Verifying,
    /// Present and verified.
    Ready,
    /// Download or verification failed.
    Failed,
}

impl FileState {
    /// Text stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Downloading => "downloading",
            Self::Verifying => "verifying",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }

    /// Parse a stored value; unknown text is `Failed` rather than hidden.
    pub fn parse(text: &str) -> Self {
        match text {
            "pending" => Self::Pending,
            "downloading" => Self::Downloading,
            "verifying" => Self::Verifying,
            "ready" => Self::Ready,
            _ => Self::Failed,
        }
    }
}

/// A file belonging to a model revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// Path inside the repository.
    pub path: String,
    /// Role: `weights`, `mmproj`, ...
    pub role: String,
    /// Whether the model needs it.
    pub required: bool,
    /// Size in bytes.
    pub size_bytes: i64,
    /// Published SHA-256.
    pub sha256: Option<String>,
    /// Current state.
    pub state: FileState,
}

/// A model revision and its files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionRecord {
    /// `publisher/name`.
    pub model_id: String,
    /// Immutable revision.
    pub revision: String,
    /// Architecture from the GGUF header.
    pub architecture: Option<String>,
    /// License identifier, when the repository states one.
    pub license: Option<String>,
    /// Files.
    pub files: Vec<FileRecord>,
    /// UTC epoch milliseconds when the revision was first recorded.
    pub recorded_at: i64,
}

impl Database {
    /// Record (or update) a model revision and its files.
    ///
    /// Runs in one transaction: a revision without its files, or files without a
    /// revision, would be a half-truth in the UI.
    pub async fn record_revision(&self, record: RevisionRecord) -> Result<(), DbError> {
        self.call(move |conn| {
            let tx = conn.transaction()?;

            let (publisher, name) = record
                .model_id
                .split_once('/')
                .map(|(p, n)| (p.to_owned(), n.to_owned()))
                .unwrap_or_else(|| ("unknown".to_owned(), record.model_id.clone()));

            tx.execute(
                "INSERT INTO models (model_id, publisher, display_name, architecture,
                                     modalities_json, license, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, '[\"text\"]', ?5, ?6, ?6)
                 ON CONFLICT (model_id) DO UPDATE SET
                     architecture = excluded.architecture,
                     license      = COALESCE(excluded.license, models.license),
                     updated_at   = excluded.updated_at",
                params![
                    record.model_id,
                    publisher,
                    name,
                    record.architecture,
                    record.license,
                    record.recorded_at,
                ],
            )?;

            let revision_id = format!("{}@{}", record.model_id, record.revision);
            tx.execute(
                "INSERT INTO model_revisions (revision_id, model_id, revision, resolved_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (revision_id) DO UPDATE SET resolved_at = excluded.resolved_at",
                params![
                    revision_id,
                    record.model_id,
                    record.revision,
                    record.recorded_at
                ],
            )?;

            for file in &record.files {
                let file_id = format!("{revision_id}:{}", file.path);
                tx.execute(
                    "INSERT INTO model_files (file_id, revision_id, path, role, required,
                                              size_bytes, sha256, state)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT (revision_id, path) DO UPDATE SET
                         role       = excluded.role,
                         required   = excluded.required,
                         size_bytes = excluded.size_bytes,
                         sha256     = excluded.sha256,
                         state      = excluded.state",
                    params![
                        file_id,
                        revision_id,
                        file.path,
                        file.role,
                        i64::from(file.required),
                        file.size_bytes,
                        file.sha256,
                        file.state.as_str(),
                    ],
                )?;
            }

            tx.commit()?;
            Ok(())
        })
        .await
    }

    /// Load a revision with its files.
    pub async fn load_revision(
        &self,
        model_id: &str,
        revision: &str,
    ) -> Result<Option<RevisionRecord>, DbError> {
        let model_id = model_id.to_owned();
        let revision = revision.to_owned();

        self.call(move |conn| {
            let revision_id = format!("{model_id}@{revision}");

            let header = conn
                .query_row(
                    "SELECT m.architecture, m.license, r.resolved_at
                     FROM model_revisions r
                     JOIN models m ON m.model_id = r.model_id
                     WHERE r.revision_id = ?1",
                    [&revision_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()?;

            let Some((architecture, license, recorded_at)) = header else {
                return Ok(None);
            };

            let mut statement = conn.prepare(
                "SELECT path, role, required, size_bytes, sha256, state
                 FROM model_files WHERE revision_id = ?1 ORDER BY path",
            )?;

            let files = statement
                .query_map([&revision_id], |row| {
                    let state: String = row.get(5)?;
                    Ok(FileRecord {
                        path: row.get(0)?,
                        role: row.get(1)?,
                        required: row.get::<_, i64>(2)? != 0,
                        size_bytes: row.get(3)?,
                        sha256: row.get(4)?,
                        state: FileState::parse(&state),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(Some(RevisionRecord {
                model_id,
                revision,
                architecture,
                license,
                files,
                recorded_at,
            }))
        })
        .await
    }

    /// Every recorded revision, newest first.
    pub async fn list_revisions(&self) -> Result<Vec<RevisionRecord>, DbError> {
        let ids = self
            .call(|conn| {
                let mut statement = conn.prepare(
                    "SELECT model_id, revision FROM model_revisions ORDER BY resolved_at DESC",
                )?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;

        let mut records = Vec::with_capacity(ids.len());
        for (model_id, revision) in ids {
            if let Some(record) = self.load_revision(&model_id, &revision).await? {
                records.push(record);
            }
        }
        Ok(records)
    }
}
