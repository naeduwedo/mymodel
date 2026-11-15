//! Registry tests: the store must never hold a half-truth.

use gguf_engine_database::{Database, FileRecord, FileState, RevisionRecord};

fn revision() -> RevisionRecord {
    RevisionRecord {
        model_id: "publisher/tiny".into(),
        revision: "0123456789abcdef".into(),
        architecture: Some("llama".into()),
        license: Some("apache-2.0".into()),
        files: vec![FileRecord {
            path: "model-Q4_K_M.gguf".into(),
            role: "weights".into(),
            required: true,
            size_bytes: 4_000_000,
            sha256: Some("a".repeat(64)),
            state: FileState::Ready,
        }],
        recorded_at: 1_700_000_000_000,
    }
}

#[tokio::test]
async fn records_and_loads_a_revision_with_its_files() {
    let db = Database::open_in_memory().unwrap();
    db.record_revision(revision()).await.unwrap();

    let loaded = db
        .load_revision("publisher/tiny", "0123456789abcdef")
        .await
        .unwrap()
        .expect("revision exists");

    assert_eq!(loaded, revision());
}

#[tokio::test]
async fn recording_the_same_revision_twice_updates_instead_of_duplicating() {
    let db = Database::open_in_memory().unwrap();
    db.record_revision(revision()).await.unwrap();

    let mut updated = revision();
    updated.files[0].state = FileState::Failed;
    updated.files[0].sha256 = None;
    db.record_revision(updated.clone()).await.unwrap();

    let loaded = db
        .load_revision("publisher/tiny", "0123456789abcdef")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.files.len(), 1);
    assert_eq!(loaded.files[0].state, FileState::Failed);
    assert_eq!(loaded.files[0].sha256, None);

    let count: i64 = db
        .call(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM model_files", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(count, 1, "upsert, not append");
}

#[tokio::test]
async fn an_unknown_revision_is_none() {
    let db = Database::open_in_memory().unwrap();
    assert!(
        db.load_revision("nobody/nothing", "main")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn listing_returns_recorded_revisions() {
    let db = Database::open_in_memory().unwrap();
    db.record_revision(revision()).await.unwrap();

    let mut second = revision();
    second.revision = "fedcba9876543210".into();
    second.recorded_at += 1000;
    db.record_revision(second).await.unwrap();

    let all = db.list_revisions().await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].revision, "fedcba9876543210", "newest first");
}

#[tokio::test]
async fn a_failed_file_state_round_trips_rather_than_being_hidden() {
    let db = Database::open_in_memory().unwrap();
    let mut record = revision();
    record.files[0].state = FileState::Failed;
    db.record_revision(record.clone()).await.unwrap();

    let loaded = db
        .load_revision(&record.model_id, &record.revision)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.files[0].state, FileState::Failed);
}

#[tokio::test]
async fn unknown_state_text_is_treated_as_failed_not_as_ready() {
    let db = Database::open_in_memory().unwrap();
    let record = revision();
    db.record_revision(record.clone()).await.unwrap();

    // Simulate a row written by a newer build with a state this one does not know.
    db.call(|conn| {
        conn.execute("UPDATE model_files SET state = 'paused'", [])?;
        Ok(())
    })
    .await
    .unwrap();

    let loaded = db
        .load_revision(&record.model_id, &record.revision)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.files[0].state,
        FileState::Failed,
        "an unrecognised state must never read as ready"
    );
}
