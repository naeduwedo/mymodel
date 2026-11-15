//! Repository for the model registry tables (spec §7.2, §10).
//!
//! Only what Task 2 needs is implemented: enough to prove the schema and the
//! storage layer work. Downloads, manifests and compatibility reports arrive
//! with Task 5 and will add to this module.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{Database, DbError};

/// A model as stored in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRecord {
    /// `publisher/name` identifier.
    pub model_id: String,
    /// Publisher namespace.
    pub publisher: String,
    /// Human readable name.
    pub display_name: String,
    /// Architecture reported by the GGUF metadata, when known.
    pub architecture: Option<String>,
    /// Modalities declared by the manifest.
    pub modalities: Vec<String>,
    /// License identifier, when the repository states one.
    pub license: Option<String>,
    /// UTC epoch milliseconds.
    pub created_at: i64,
    /// UTC epoch milliseconds.
    pub updated_at: i64,
}

impl Database {
    /// Insert or update a model record.
    pub async fn upsert_model(&self, model: ModelRecord) -> Result<(), DbError> {
        self.call(move |conn| {
            conn.execute(
                "INSERT INTO models (
                     model_id, publisher, display_name, architecture,
                     modalities_json, license, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (model_id) DO UPDATE SET
                     publisher     = excluded.publisher,
                     display_name  = excluded.display_name,
                     architecture  = excluded.architecture,
                     modalities_json = excluded.modalities_json,
                     license       = excluded.license,
                     updated_at    = excluded.updated_at",
                params![
                    model.model_id,
                    model.publisher,
                    model.display_name,
                    model.architecture,
                    serde_json::to_string(&model.modalities)?,
                    model.license,
                    model.created_at,
                    model.updated_at,
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// Fetch one model, or `None`.
    pub async fn get_model(&self, model_id: &str) -> Result<Option<ModelRecord>, DbError> {
        let id = model_id.to_owned();
        self.call(move |conn| {
            let row = conn
                .query_row(
                    "SELECT model_id, publisher, display_name, architecture,
                            modalities_json, license, created_at, updated_at
                     FROM models WHERE model_id = ?1 AND deleted_at IS NULL",
                    [&id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                        ))
                    },
                )
                .optional()?;
            match row {
                Some((
                    id,
                    publisher,
                    display_name,
                    architecture,
                    json,
                    license,
                    created,
                    updated,
                )) => Ok(Some(ModelRecord {
                    model_id: id,
                    publisher,
                    display_name,
                    architecture,
                    modalities: serde_json::from_str(&json)?,
                    license,
                    created_at: created,
                    updated_at: updated,
                })),
                None => Ok(None),
            }
        })
        .await
    }

    /// Count of registered models.
    pub async fn model_count(&self) -> Result<i64, DbError> {
        self.call(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM models WHERE deleted_at IS NULL",
                [],
                |row| row.get(0),
            )?)
        })
        .await
    }

    /// Soft-delete a model; the row stays for audit purposes.
    pub async fn soft_delete_model(&self, model_id: &str, at: i64) -> Result<bool, DbError> {
        let id = model_id.to_owned();
        self.call(move |conn| {
            let changed = conn.execute(
                "UPDATE models SET deleted_at = ?2 WHERE model_id = ?1 AND deleted_at IS NULL",
                params![id, at],
            )?;
            Ok(changed > 0)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> ModelRecord {
        ModelRecord {
            model_id: id.to_owned(),
            publisher: "publisher".into(),
            display_name: "Tiny Model".into(),
            architecture: Some("llama".into()),
            modalities: vec!["text".into()],
            license: Some("apache-2.0".into()),
            created_at: 1,
            updated_at: 1,
        }
    }

    #[tokio::test]
    async fn round_trips_a_model() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_model(model("publisher/tiny")).await.unwrap();
        let stored = db.get_model("publisher/tiny").await.unwrap().unwrap();
        assert_eq!(stored, model("publisher/tiny"));
        assert_eq!(db.model_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn upsert_updates_instead_of_duplicating() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_model(model("publisher/tiny")).await.unwrap();
        let mut updated = model("publisher/tiny");
        updated.display_name = "Renamed".into();
        updated.updated_at = 2;
        db.upsert_model(updated.clone()).await.unwrap();
        assert_eq!(db.model_count().await.unwrap(), 1);
        assert_eq!(
            db.get_model("publisher/tiny").await.unwrap().unwrap(),
            updated
        );
    }

    #[tokio::test]
    async fn soft_deleted_models_are_hidden_but_retained() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_model(model("publisher/tiny")).await.unwrap();
        assert!(db.soft_delete_model("publisher/tiny", 5).await.unwrap());
        assert!(db.get_model("publisher/tiny").await.unwrap().is_none());
        assert_eq!(db.model_count().await.unwrap(), 0);
        assert!(!db.soft_delete_model("publisher/tiny", 6).await.unwrap());
        let rows: i64 = db
            .call(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM models", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(rows, 1, "the row is kept for audit");
    }

    #[tokio::test]
    async fn missing_model_is_none() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.get_model("nobody/nothing").await.unwrap().is_none());
    }
}
