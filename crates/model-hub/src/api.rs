//! Hugging Face Hub API client (spec §7.3).
//!
//! Only the public HTTP API is used: no Hub Python library, no repository code is
//! ever executed (spec rule 8). Every field that ends up in a manifest comes from
//! this API rather than from a file name.
//!
//! # Token handling
//!
//! A token is optional and is passed in as a value the caller owns. It is never
//! logged: the client prints only whether one was supplied (spec §9.3).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::error::HubError;

/// Default Hub endpoint.
pub const DEFAULT_ENDPOINT: &str = "https://huggingface.co";

/// How many results a search returns by default.
pub const DEFAULT_SEARCH_LIMIT: usize = 20;

/// A repository summary from a search.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RepoSummary {
    /// `publisher/name`.
    pub id: String,
    /// Default branch commit, when the API reports one.
    #[serde(default)]
    pub sha: Option<String>,
    /// Likes; a rough popularity signal.
    #[serde(default)]
    pub likes: u64,
    /// Download count.
    #[serde(default)]
    pub downloads: u64,
    /// Whether the repository is gated.
    #[serde(default)]
    pub gated: bool,
    /// Tags (includes `gguf` for GGUF repositories).
    #[serde(default)]
    pub tags: Vec<String>,
}

impl RepoSummary {
    /// True when the repository is tagged as containing GGUF files.
    pub fn is_gguf(&self) -> bool {
        self.tags.iter().any(|tag| tag.eq_ignore_ascii_case("gguf"))
    }
}

/// One entry from a repository tree listing.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TreeEntry {
    /// File path inside the repository.
    pub path: String,
    /// `file` or `directory`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Size in bytes (absent for directories).
    #[serde(default)]
    pub size: u64,
    /// Git LFS pointer details, when the file is stored in LFS.
    #[serde(default)]
    pub lfs: Option<LfsInfo>,
}

impl TreeEntry {
    /// True for a regular file.
    pub fn is_file(&self) -> bool {
        self.kind == "file"
    }

    /// True when the path ends in `.gguf` (case-insensitive).
    pub fn is_gguf(&self) -> bool {
        self.path.to_ascii_lowercase().ends_with(".gguf")
    }

    /// The SHA-256 the Hub publishes for this file, if any.
    ///
    /// For LFS files this is the real content hash. For small non-LFS files the
    /// API reports a git blob SHA-1, which is **not** a content checksum, so it is
    /// deliberately not returned here.
    pub fn sha256(&self) -> Option<&str> {
        self.lfs.as_ref().map(|lfs| lfs.sha256.as_str())
    }
}

/// Git LFS metadata.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct LfsInfo {
    /// SHA-256 of the file content, as published by the Hub.
    #[serde(rename = "oid")]
    pub sha256: String,
    /// Size in bytes.
    #[serde(default)]
    pub size: u64,
}

/// A configured Hub client.
///
/// Debug is implemented by hand: the token must never reach a log line, and a
/// derived Debug would print it the first time someone logged the client.
#[derive(Clone)]
pub struct HubClient {
    http: reqwest::Client,
    endpoint: String,
    token: Option<String>,
}

impl HubClient {
    /// Create a client against the public Hub.
    pub fn new(token: Option<String>) -> Result<Self, HubError> {
        Self::with_endpoint(DEFAULT_ENDPOINT, token)
    }

    /// Create a client against an explicit endpoint (used by tests and mirrors).
    pub fn with_endpoint(
        endpoint: impl Into<String>,
        token: Option<String>,
    ) -> Result<Self, HubError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("gguf-ai-engine/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()?;

        Ok(Self {
            http,
            endpoint: endpoint.into().trim_end_matches("/").to_owned(),
            token,
        })
    }

    /// Endpoint in use.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Whether a token was supplied (never the token itself).
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn request(&self, url: &str) -> reqwest::RequestBuilder {
        let request = self.http.get(url);
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, HubError> {
        debug!(%url, "hub request");
        let response = self.request(url).send().await?;

        match response.status().as_u16() {
            200..=299 => {}
            401 | 403 => return Err(HubError::Unauthorized),
            status => {
                return Err(HubError::Status {
                    status,
                    url: url.to_owned(),
                });
            }
        }

        let body = response.text().await?;
        Ok(serde_json::from_str(&body)?)
    }

    /// Search repositories.
    ///
    /// `filter` is applied by the Hub itself (e.g. `gguf`), so results do not have
    /// to be filtered after the fact.
    pub async fn search(
        &self,
        query: &str,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RepoSummary>, HubError> {
        let mut url = format!(
            "{}/api/models?search={}&limit={}",
            self.endpoint,
            urlencode(query),
            limit.clamp(1, 100)
        );
        if let Some(filter) = filter {
            url.push_str("&filter=");
            url.push_str(&urlencode(filter));
        }

        self.get_json(&url).await
    }

    /// List the files of a repository at a pinned revision.
    pub async fn tree(&self, repo: &str, revision: &str) -> Result<Vec<TreeEntry>, HubError> {
        let url = format!(
            "{}/api/models/{}/tree/{}?recursive=true",
            self.endpoint, repo, revision
        );

        self.get_json(&url).await
    }

    /// Repository metadata, including the resolved commit for a revision.
    pub async fn repo_info(&self, repo: &str, revision: &str) -> Result<RepoInfo, HubError> {
        let url = format!(
            "{}/api/models/{}?revision={}",
            self.endpoint, repo, revision
        );

        self.get_json(&url).await
    }

    /// URL to download one file at a pinned revision.
    pub fn download_url(&self, repo: &str, revision: &str, path: &str) -> String {
        format!("{}/{}/resolve/{}/{}", self.endpoint, repo, revision, path)
    }

    /// Raw HTTP client, for the downloader.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Bearer token, if any.
    pub(crate) fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }
}

impl std::fmt::Debug for HubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubClient")
            .field("endpoint", &self.endpoint)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Repository metadata.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RepoInfo {
    /// `publisher/name`.
    pub id: String,
    /// Commit SHA the requested revision resolved to.
    #[serde(default)]
    pub sha: Option<String>,
    /// License identifier, when the repository declares a conventional one.
    #[serde(default)]
    pub card_data: Option<serde_json::Value>,
    /// Tags.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl RepoInfo {
    /// License from the model card, when it is a plain string.
    pub fn license(&self) -> Option<String> {
        let card = self.card_data.as_ref()?;
        let value = card.get("license")?;
        match value {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Array(items) => items
                .first()
                .and_then(|item| item.as_str())
                .map(str::to_owned),
            _ => None,
        }
    }
}

/// Percent-encode the characters that matter in a query value.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_results_deserialize_and_flag_gguf() {
        let json = r#"[{"id":"a/b","sha":"abc","likes":5,"downloads":10,
                       "gated":false,"tags":["gguf","text-generation"]}]"#;
        let repos: Vec<RepoSummary> = serde_json::from_str(json).unwrap();
        assert_eq!(repos[0].id, "a/b");
        assert!(repos[0].is_gguf());
    }

    #[test]
    fn tree_entries_expose_the_lfs_hash_only_for_lfs_files() {
        let lfs = r#"{"path":"m.gguf","type":"file","size":10,
                     "lfs":{"oid":"deadbeef","size":10}}"#;
        let entry: TreeEntry = serde_json::from_str(lfs).unwrap();
        assert!(entry.is_file());
        assert!(entry.is_gguf());
        assert_eq!(entry.sha256(), Some("deadbeef"));

        // A small non-LFS file reports a git blob id, which is not a content hash.
        let plain = r#"{"path":"config.json","type":"file","size":5}"#;
        let entry: TreeEntry = serde_json::from_str(plain).unwrap();
        assert!(entry.is_file());
        assert_eq!(entry.sha256(), None, "no LFS oid means no verifiable hash");
        assert!(!entry.is_gguf());
    }

    #[test]
    fn download_urls_pin_the_revision() {
        let client = HubClient::with_endpoint("https://example.test/", None).unwrap();
        assert_eq!(client.endpoint(), "https://example.test");
        assert_eq!(
            client.download_url("a/b", "c0ffee", "model-Q4_K_M.gguf"),
            "https://example.test/a/b/resolve/c0ffee/model-Q4_K_M.gguf"
        );
    }

    #[test]
    fn query_values_are_encoded() {
        assert_eq!(urlencode("qwen 2.5"), "qwen%202.5");
        assert_eq!(urlencode("gguf"), "gguf");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn a_token_is_never_exposed_but_its_presence_is() {
        let client =
            HubClient::with_endpoint("https://example.test", Some("secret".into())).unwrap();
        assert!(client.has_token());
        assert!(
            !format!("{client:?}").contains("secret"),
            "Debug must not leak it"
        );
    }

    #[test]
    fn license_is_read_from_card_data_only_when_present() {
        let info = RepoInfo {
            id: "a/b".into(),
            sha: None,
            card_data: Some(serde_json::json!({"license": "apache-2.0"})),
            tags: vec![],
        };
        assert_eq!(info.license().as_deref(), Some("apache-2.0"));

        let none = RepoInfo {
            id: "a/b".into(),
            sha: None,
            card_data: None,
            tags: vec![],
        };
        assert_eq!(
            none.license(),
            None,
            "no licence must stay unknown, not guessed"
        );
    }
}
