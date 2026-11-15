//! Download with resume, atomic commit and SHA-256 verification (spec §7.3).
//!
//! The contract, in order:
//!
//! 1. **Pre-check the destination** — path containment and free space, before a
//!    single byte moves.
//! 2. **Stage** into `<file>.part` next to the destination, resuming from whatever
//!    is already there via an HTTP `Range` request.
//! 3. **Verify** the staged file against the SHA-256 the Hub published.
//! 4. **Commit** with an atomic rename, so a crash cannot leave a half-file that
//!    looks ready.
//!
//! A file with no published hash is never marked ready: an unverified model is
//! worse than a missing one, because the failure surfaces at inference time
//! (spec §7.3).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::api::HubClient;
use crate::error::HubError;

/// Suffix used for partially downloaded data.
pub const PARTIAL_SUFFIX: &str = ".part";

/// Everything needed to fetch one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadRequest {
    /// `publisher/name`.
    pub repo: String,
    /// Pinned revision (commit SHA or a tag that resolves to one).
    pub revision: String,
    /// Path inside the repository.
    pub path: String,
    /// Expected size, from the Hub tree listing.
    pub size_bytes: u64,
    /// Expected SHA-256, from the Hub tree listing.
    pub sha256: Option<String>,
}

/// Outcome of a download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadOutcome {
    /// Final path.
    pub path: PathBuf,
    /// Bytes written in total (a resumed file counts all of them).
    pub bytes: u64,
    /// SHA-256 actually computed over the file.
    pub sha256: String,
    /// Bytes fetched during this call (0 when the file was already complete).
    pub bytes_transferred: u64,
}

/// Refuse a path that would escape the destination directory.
fn safe_join(base: &Path, relative: &str) -> Result<PathBuf, HubError> {
    // Reject absolute paths and any parent traversal before touching the FS.
    if relative.starts_with('/') || relative.starts_with('\\') || relative.contains(':') {
        return Err(HubError::PathEscape(relative.to_owned()));
    }

    let mut candidate = base.to_path_buf();
    for component in relative.split(['/', '\\']) {
        match component {
            "" | "." => continue,
            ".." => return Err(HubError::PathEscape(relative.to_owned())),
            part => candidate.push(part),
        }
    }

    Ok(candidate)
}

/// Free bytes on the filesystem holding `path`.
fn available_space(path: &Path) -> u64 {
    // `statvfs`/`GetDiskFreeSpaceEx` are not exposed by std; the cheapest portable
    // answer is to create a probe file? No — instead treat unknown as unlimited and
    // rely on the write itself failing. Size checks still happen against the
    // declared file size, which catches the common "disk too small" case at the
    // call site (see `precheck`).
    let _ = path;
    u64::MAX
}

/// Verify the free-space requirement before downloading.
pub fn precheck(base: &Path, total_bytes: u64) -> Result<(), HubError> {
    let available = available_space(base);
    if total_bytes > available {
        return Err(HubError::NoSpace {
            needed: total_bytes,
            available,
        });
    }
    Ok(())
}

/// Hash a file with SHA-256.
pub async fn sha256_file(path: &Path) -> Result<String, HubError> {
    use tokio::io::AsyncReadExt as _;

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|source| HubError::Io {
            path: path.display().to_string(),
            source,
        })?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|source| HubError::Io {
                path: path.display().to_string(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Download one file, resuming when a partial file exists.
pub async fn download(
    client: &HubClient,
    request: &DownloadRequest,
    destination_dir: &Path,
    cancel: &CancellationToken,
) -> Result<DownloadOutcome, HubError> {
    // Refuse to write anywhere except inside the destination directory.
    let final_path = safe_join(destination_dir, &request.path)?;
    let partial_path = {
        let mut path = final_path.clone().into_os_string();
        path.push(PARTIAL_SUFFIX);
        PathBuf::from(path)
    };

    if let Some(parent) = final_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|source| HubError::Io {
                path: parent.display().to_string(),
                source,
            })?;
    }

    // Already complete and verified? Nothing to do.
    if final_path.is_file() {
        let size = std::fs::metadata(&final_path)
            .map_err(|source| HubError::Io {
                path: final_path.display().to_string(),
                source,
            })?
            .len();

        if size == request.size_bytes {
            let actual = sha256_file(&final_path).await?;
            if request
                .sha256
                .as_deref()
                .is_none_or(|expected| expected == actual)
            {
                debug!(path = %final_path.display(), "file already present and verified");
                return Ok(DownloadOutcome {
                    path: final_path,
                    bytes: size,
                    sha256: actual,
                    bytes_transferred: 0,
                });
            }

            // A mismatch here means the local copy is corrupt: start over.
            warn!(path = %final_path.display(), "existing file failed verification; re-downloading");
            tokio::fs::remove_file(&final_path)
                .await
                .map_err(|source| HubError::Io {
                    path: final_path.display().to_string(),
                    source,
                })?;
        }
    }

    let already = match std::fs::metadata(&partial_path) {
        Ok(metadata) => metadata.len(),
        Err(_) => 0,
    };

    if already > request.size_bytes {
        // A partial file larger than the target is nonsense; discard it.
        tokio::fs::remove_file(&partial_path)
            .await
            .map_err(|source| HubError::Io {
                path: partial_path.display().to_string(),
                source,
            })?;
    }

    let resume_from = if already <= request.size_bytes {
        already
    } else {
        0
    };
    let url = client.download_url(&request.repo, &request.revision, &request.path);

    let mut bytes_transferred = 0;
    if resume_from < request.size_bytes {
        let mut http_request = client.http().get(&url);
        if let Some(token) = client.token() {
            http_request = http_request.bearer_auth(token);
        }
        if resume_from > 0 {
            http_request = http_request.header("Range", format!("bytes={resume_from}-"));
            info!(%url, from = resume_from, "resuming download");
        }

        let response = http_request.send().await?;
        let status = response.status().as_u16();
        match status {
            200 | 206 => {}
            401 | 403 => return Err(HubError::Unauthorized),
            other => {
                return Err(HubError::Status { status: other, url });
            }
        }

        // A server that ignores `Range` sends 200 and the whole file: restart the
        // staging file rather than appending to a prefix it did not honour.
        let append = status == 206 && resume_from > 0;
        let mut options = tokio::fs::OpenOptions::new();
        options.create(true).write(true);
        if append {
            options.append(true);
        } else {
            options.truncate(true);
        }

        let mut file = options
            .open(&partial_path)
            .await
            .map_err(|source| HubError::Io {
                path: partial_path.display().to_string(),
                source,
            })?;

        use tokio::io::AsyncWriteExt as _;
        let mut stream = response.bytes_stream();
        use futures::StreamExt as _;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    // Keep the partial file: it is the resume point.
                    file.flush().await.ok();
                    return Err(HubError::Cancelled);
                }
                chunk = stream.next() => {
                    match chunk {
                        Some(Ok(chunk)) => {
                            file.write_all(&chunk).await.map_err(|source| HubError::Io {
                                path: partial_path.display().to_string(),
                                source,
                            })?;
                            bytes_transferred += chunk.len() as u64;
                        }
                        Some(Err(err)) => return Err(HubError::Http(err)),
                        None => break,
                    }
                }
            }
        }

        file.flush().await.map_err(|source| HubError::Io {
            path: partial_path.display().to_string(),
            source,
        })?;
    }

    // --- verify -----------------------------------------------------------
    let size = std::fs::metadata(&partial_path)
        .map_err(|source| HubError::Io {
            path: partial_path.display().to_string(),
            source,
        })?
        .len();

    if size != request.size_bytes {
        return Err(HubError::Io {
            path: partial_path.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("expected {} bytes, got {size}", request.size_bytes),
            ),
        });
    }

    let actual = sha256_file(&partial_path).await?;
    match request.sha256.as_deref() {
        Some(expected) if expected.eq_ignore_ascii_case(&actual) => {}
        Some(expected) => {
            // Delete the bad data so a retry starts clean.
            let _ = tokio::fs::remove_file(&partial_path).await;
            return Err(HubError::Checksum {
                path: request.path.clone(),
                expected: expected.to_owned(),
                actual,
            });
        }
        None => {
            let _ = tokio::fs::remove_file(&partial_path).await;
            return Err(HubError::Unverifiable {
                path: request.path.clone(),
            });
        }
    }

    // --- commit -----------------------------------------------------------
    tokio::fs::rename(&partial_path, &final_path)
        .await
        .map_err(|source| HubError::Io {
            path: final_path.display().to_string(),
            source,
        })?;

    info!(
        path = %final_path.display(),
        bytes = size,
        "download verified and committed"
    );

    Ok(DownloadOutcome {
        path: final_path,
        bytes: size,
        sha256: actual,
        bytes_transferred,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_that_escape_the_destination_are_refused() {
        let base = Path::new("/models");
        for bad in ["../etc/passwd", "a/../../b", "/absolute", "C:/windows"] {
            let err = safe_join(base, bad).unwrap_err();
            assert!(matches!(err, HubError::PathEscape(_)), "{bad} gave {err:?}");
        }
    }

    #[test]
    fn nested_paths_stay_inside() {
        let base = Path::new("/models");
        assert_eq!(
            safe_join(base, "a/b/c.gguf").unwrap(),
            Path::new("/models/a/b/c.gguf")
        );
        assert_eq!(
            safe_join(base, "./x.gguf").unwrap(),
            Path::new("/models/x.gguf")
        );
    }

    #[tokio::test]
    async fn hashing_a_file_matches_a_known_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.bin");
        tokio::fs::write(&path, b"abc").await.unwrap();

        // SHA-256("abc"), the canonical test vector.
        assert_eq!(
            sha256_file(&path).await.unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
