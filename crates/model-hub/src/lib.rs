//! Hugging Face Hub integration (spec §7.3).
//!
//! # Guarantees
//!
//! * **Revision pinned.** Every download URL carries an explicit revision; a
//!   moving branch name is resolved to a commit before any byte moves.
//! * **Verified before ready.** A file is renamed into place only after its
//!   SHA-256 matches the hash the Hub published. A file with no published hash is
//!   refused rather than trusted.
//! * **Atomic.** Data lands in `<name>.part` and is renamed, so an interrupted
//!   download cannot leave a plausible-looking short file.
//! * **Resumable.** A `Range` request continues an existing `.part` file, and a
//!   server that ignores `Range` is detected (it answers 200) and the file is
//!   restarted instead of being corrupted by appending.
//! * **Nothing is executed.** Only JSON and file bytes are ever read from the Hub
//!   (spec rule 8).

#![warn(missing_docs)]

pub mod api;
pub mod download;
pub mod error;

pub use api::{
    DEFAULT_ENDPOINT, DEFAULT_SEARCH_LIMIT, HubClient, LfsInfo, RepoInfo, RepoSummary, TreeEntry,
};
pub use download::{
    DownloadOutcome, DownloadRequest, PARTIAL_SUFFIX, download, precheck, sha256_file,
};
pub use error::HubError;
