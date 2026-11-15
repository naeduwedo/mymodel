//! `gguf models` - search, list and pull from the Hub (spec §4.3, §7.3).
//!
//! Every command here is explicit about what it did: a pull prints the resolved
//! revision, the file, its size and its verified hash. A model appears in `list`
//! only after its bytes were verified.

use anyhow::{Context as _, Result};
use gguf_engine_database::{Database, FileRecord, FileState, RevisionRecord};
use gguf_engine_model_hub::{DownloadRequest, HubClient, TreeEntry, download};
use tokio_util::sync::CancellationToken;

use crate::commands::Context;
use crate::output;

/// Search the Hub.
pub async fn search(ctx: &Context, query: &str, limit: usize, json: bool) -> Result<bool> {
    let client = HubClient::new(None).context("creating the Hub client")?;
    let repos = client
        .search(query, Some("gguf"), limit)
        .await
        .context("searching the Hub")?;

    if json {
        println!("{}", serde_json::to_string_pretty(&repos)?);
        return Ok(true);
    }

    if repos.is_empty() {
        println!("no GGUF repositories matched {query:?}");
        return Ok(true);
    }

    println!("{:<60} {:>10} {:>10}", "repository", "downloads", "likes");
    for repo in &repos {
        println!("{:<60} {:>10} {:>10}", repo.id, repo.downloads, repo.likes);
    }
    let _ = ctx;
    Ok(true)
}

/// List registered models.
pub fn list(ctx: &Context, json: bool) -> Result<bool> {
    let db = Database::open(ctx.paths.database_file()).context("opening the registry")?;
    let revisions = db.call_blocking(|conn| {
        // A blocking helper keeps this command synchronous; the registry is tiny.
        let mut statement = conn.prepare(
            "SELECT r.model_id, r.revision, m.architecture, m.license
             FROM model_revisions r JOIN models m ON m.model_id = r.model_id
             ORDER BY r.resolved_at DESC",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(RevisionRecord {
                    model_id: row.get(0)?,
                    revision: row.get(1)?,
                    architecture: row.get(2)?,
                    license: row.get(3)?,
                    files: Vec::new(),
                    recorded_at: 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;

    if json {
        println!("{}", serde_json::to_string_pretty(&revisions)?);
        return Ok(true);
    }

    if revisions.is_empty() {
        println!("no models registered yet");
        println!("pull one with: gguf models pull <repo> --file <path.gguf>");
        return Ok(true);
    }

    println!(
        "{:<45} {:<14} {:<12} license",
        "model", "revision", "architecture"
    );
    for record in &revisions {
        println!(
            "{:<45} {:<14} {:<12} {}",
            record.model_id,
            &record.revision[..record.revision.len().min(12)],
            record.architecture.as_deref().unwrap_or("-"),
            record.license.as_deref().unwrap_or("-")
        );
    }
    Ok(true)
}

/// Pull one file from a repository, verify it and register it.
pub async fn pull(
    ctx: &Context,
    repo: &str,
    file: Option<&str>,
    revision: &str,
    json: bool,
) -> Result<bool> {
    let client = HubClient::new(None).context("creating the Hub client")?;

    // Resolve the revision to a commit first: a branch name can move between the
    // listing and the download, and the manifest must record something immutable.
    let info = client
        .repo_info(repo, revision)
        .await
        .with_context(|| format!("reading repository info for {repo}"))?;
    let commit = info.sha.clone().unwrap_or_else(|| revision.to_owned());

    let tree = client
        .tree(repo, &commit)
        .await
        .with_context(|| format!("listing files of {repo}@{commit}"))?;

    let candidates: Vec<&TreeEntry> = tree.iter().filter(|entry| entry.is_gguf()).collect();
    if candidates.is_empty() {
        anyhow::bail!("{repo}@{commit} contains no .gguf files");
    }

    let chosen = match file {
        Some(wanted) => candidates
            .iter()
            .find(|entry| entry.path == wanted)
            .copied()
            .with_context(|| format!("{wanted} is not present in {repo}@{commit}"))?,
        None => {
            if candidates.len() > 1 {
                println!("{repo}@{commit} contains {} GGUF files:", candidates.len());
                for entry in &candidates {
                    println!("  {:<60} {}", entry.path, output::human_bytes(entry.size));
                }
                anyhow::bail!("choose one with --file <path>");
            }
            candidates[0]
        }
    };

    let Some(expected) = chosen.sha256().map(str::to_owned) else {
        anyhow::bail!(
            "{} has no published SHA-256, so it cannot be verified and will not be marked ready",
            chosen.path
        );
    };

    let models_dir = ctx.paths.models().to_path_buf();
    tokio::fs::create_dir_all(&models_dir)
        .await
        .context("creating the models directory")?;

    let request = DownloadRequest {
        repo: repo.to_owned(),
        revision: commit.clone(),
        path: chosen.path.clone(),
        size_bytes: chosen.size,
        sha256: Some(expected.clone()),
    };

    if !json {
        println!("repository : {repo}");
        println!("revision   : {commit}");
        println!(
            "file       : {} ({})",
            chosen.path,
            output::human_bytes(chosen.size)
        );
        println!(
            "verifying  : sha256 {}",
            &expected[..expected.len().min(16)]
        );
    }

    let cancel = CancellationToken::new();
    let outcome = download(&client, &request, &models_dir, &cancel)
        .await
        .context("downloading the model file")?;

    // The file is now on disk and verified. Parsing its header (a bounded
    // prefix read, not the whole file) recovers the real architecture so the
    // registry never stores a guess.
    let architecture = gguf_engine_gguf::parse_file(&outcome.path)
        .ok()
        .and_then(|gguf| gguf.architecture().map(str::to_owned));

    // Register only what was verified.
    let record = RevisionRecord {
        model_id: repo.to_owned(),
        revision: commit.clone(),
        architecture,
        license: info.license(),
        files: vec![FileRecord {
            path: chosen.path.clone(),
            role: "weights".to_owned(),
            required: true,
            size_bytes: i64::try_from(outcome.bytes).unwrap_or(i64::MAX),
            sha256: Some(outcome.sha256.clone()),
            state: FileState::Ready,
        }],
        recorded_at: gguf_engine_hardware_time::now_ms(),
    };

    let db = Database::open(ctx.paths.database_file()).context("opening the registry")?;
    // pull is already async: record directly, never block the runtime.
    db.record_revision(record)
        .await
        .context("recording the model")?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "repo": repo,
                "revision": commit,
                "file": chosen.path,
                "path": outcome.path.display().to_string(),
                "bytes": outcome.bytes,
                "sha256": outcome.sha256,
            })
        );
    } else {
        println!("verified   : sha256 {}", &outcome.sha256[..16]);
        println!("stored     : {}", outcome.path.display());
        println!();
        println!("registered in the model registry");
    }

    Ok(true)
}

/// Small helper so this module does not depend on the hardware crate for a clock.
mod gguf_engine_hardware_time {
    pub fn now_ms() -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or_default()
    }
}
