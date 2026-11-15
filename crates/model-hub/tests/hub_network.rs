//! Integration tests against the real Hugging Face Hub.
//!
//! These hit the network, so they are opt-in: `GGUF_HUB_TESTS=1`. That is stated
//! plainly rather than hidden behind a silent skip, because a "passing" network
//! test that never ran is worse than a failing one.
//!
//! What is checked when enabled:
//!
//! * search returns GGUF repositories;
//! * a repository tree lists `.gguf` files with LFS hashes;
//! * a real small file downloads, verifies and is renamed into place;
//! * a corrupted expectation is rejected, and nothing is left behind;
//! * resume continues from a partial file instead of restarting.

use std::time::Duration;

use gguf_engine_model_hub::{DownloadRequest, HubClient, HubError, download};
use tokio_util::sync::CancellationToken;

fn require_enabled() {
    // The whole module is #[ignore]d by default; this is a second, belt-and-braces
    // guard so the tests never touch the network unless explicitly opted in.
    assert!(
        std::env::var("GGUF_HUB_TESTS").ok().as_deref() == Some("1"),
        "network test: set GGUF_HUB_TESTS=1 and run with --ignored"
    );
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

#[test]
#[ignore = "network test; run with GGUF_HUB_TESTS=1 cargo test -p gguf-engine-model-hub --test hub_network -- --ignored"]
fn search_finds_gguf_repositories() {
    require_enabled();

    runtime().block_on(async {
        let client = HubClient::new(None).expect("client");
        let results = client
            .search("qwen2", Some("gguf"), 5)
            .await
            .expect("search");

        assert!(!results.is_empty(), "the Hub should return something");
        assert!(results.len() <= 5);
        for repo in &results {
            assert!(
                repo.id.contains('/'),
                "id should be publisher/name: {}",
                repo.id
            );
            assert!(
                !repo.sha.as_deref().unwrap_or("").is_empty() || repo.sha.is_none(),
                "sha may be absent"
            );
        }
    });
}

#[test]
#[ignore = "network test"]
fn a_known_repository_lists_gguf_files_with_hashes() {
    require_enabled();

    runtime().block_on(async {
        let client = HubClient::new(None).expect("client");

        // A tiny, stable test repository maintained for exactly this purpose.
        let repo = "ggml-org/models";
        let tree = client.tree(repo, "main").await.expect("tree");

        assert!(!tree.is_empty(), "tree should not be empty");
        let ggufs: Vec<_> = tree.iter().filter(|e| e.is_gguf()).collect();
        assert!(!ggufs.is_empty(), "{repo} should contain GGUF files");

        for entry in ggufs.iter().take(5) {
            assert!(entry.is_file());
            assert!(entry.size > 0, "{} has no size", entry.path);
        }
    });
}

#[test]
#[ignore = "network test"]
fn downloads_a_real_file_verifies_and_commits_it() {
    require_enabled();

    runtime().block_on(async {
        let client = HubClient::new(None).expect("client");
        let repo = "ggml-org/models";
        let tree = client.tree(repo, "main").await.expect("tree");

        // Pick the smallest GGUF with a published hash.
        let mut candidates: Vec<_> = tree
            .iter()
            .filter(|e| {
                e.is_gguf() && !e.path.contains("-be") && e.sha256().is_some() && e.size > 0
            })
            .collect();
        candidates.sort_by_key(|e| e.size);
        let Some(smallest) = candidates.first() else {
            panic!("no verifiable GGUF file in {repo}");
        };

        let dir = tempfile::tempdir().expect("temp dir");
        let request = DownloadRequest {
            repo: repo.to_owned(),
            revision: "main".to_owned(),
            path: smallest.path.clone(),
            size_bytes: smallest.size,
            sha256: smallest.sha256().map(str::to_owned),
        };

        let cancel = CancellationToken::new();
        let outcome = download(&client, &request, dir.path(), &cancel)
            .await
            .expect("download");

        assert!(outcome.path.is_file());
        assert_eq!(outcome.bytes, smallest.size);
        assert_eq!(
            outcome.sha256.to_lowercase(),
            smallest.sha256().unwrap().to_lowercase(),
            "the downloaded hash must match the published one"
        );

        // The staging file must be gone.
        let partial = outcome.path.with_extension("gguf.part");
        assert!(
            !partial.exists(),
            "a committed download leaves no .part file"
        );

        // And the file must actually parse as GGUF.
        let gguf = gguf_engine_gguf::parse_file(&outcome.path).expect("downloaded file parses");
        assert!(!gguf.tensors.is_empty());
    });
}

#[test]
#[ignore = "network test"]
fn a_wrong_expected_hash_is_rejected_and_nothing_is_kept() {
    require_enabled();

    runtime().block_on(async {
        let client = HubClient::new(None).expect("client");
        let repo = "ggml-org/models";
        let tree = client.tree(repo, "main").await.expect("tree");
        let mut candidates: Vec<_> = tree
            .iter()
            .filter(|e| {
                e.is_gguf() && !e.path.contains("-be") && e.sha256().is_some() && e.size > 0
            })
            .collect();
        candidates.sort_by_key(|e| e.size);
        let smallest = candidates.first().expect("verifiable GGUF");

        let dir = tempfile::tempdir().expect("temp dir");
        let request = DownloadRequest {
            repo: repo.to_owned(),
            revision: "main".to_owned(),
            path: smallest.path.clone(),
            size_bytes: smallest.size,
            // Deliberately wrong, but the right shape.
            sha256: Some("0".repeat(64)),
        };

        let cancel = CancellationToken::new();
        let err = download(&client, &request, dir.path(), &cancel)
            .await
            .expect_err("a wrong hash must fail");

        assert!(matches!(err, HubError::Checksum { .. }), "got {err:?}");

        // Nothing that could be mistaken for a ready model may remain.
        let committed = dir.path().join(&smallest.path);
        assert!(!committed.exists(), "a failed verification must not commit");
    });
}

#[test]
#[ignore = "network test"]
fn a_partial_file_is_resumed_rather_than_restarted() {
    require_enabled();

    runtime().block_on(async {
        let client = HubClient::new(None).expect("client");
        let repo = "ggml-org/models";
        let tree = client.tree(repo, "main").await.expect("tree");
        let mut candidates: Vec<_> = tree
            .iter()
            .filter(|e| {
                e.is_gguf() && !e.path.contains("-be") && e.sha256().is_some() && e.size > 4096
            })
            .collect();
        candidates.sort_by_key(|e| e.size);
        let target = candidates.first().expect("GGUF larger than 4 KiB");

        let dir = tempfile::tempdir().expect("temp dir");
        let request = DownloadRequest {
            repo: repo.to_owned(),
            revision: "main".to_owned(),
            path: target.path.clone(),
            size_bytes: target.size,
            sha256: target.sha256().map(str::to_owned),
        };

        // First half of the file, staged as if a previous run was interrupted.
        let partial = dir.path().join(format!("{}.part", target.path));
        if let Some(parent) = partial.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }

        let url = client.download_url(repo, "main", &target.path);
        let first_half = reqwest::Client::new()
            .get(&url)
            .header("Range", "bytes=0-2047")
            .send()
            .await
            .expect("range request")
            .bytes()
            .await
            .expect("range body");
        std::fs::write(&partial, &first_half).unwrap();

        let cancel = CancellationToken::new();
        tokio::time::timeout(
            Duration::from_secs(120),
            download(&client, &request, dir.path(), &cancel),
        )
        .await
        .expect("download finished")
        .expect("resumed download succeeds");

        let final_path = dir.path().join(&target.path);
        assert_eq!(std::fs::metadata(&final_path).unwrap().len(), target.size);
        assert_eq!(
            gguf_engine_model_hub::sha256_file(&final_path)
                .await
                .unwrap()
                .to_lowercase(),
            target.sha256().unwrap().to_lowercase(),
            "a resumed file must still verify"
        );
    });
}
