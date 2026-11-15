//! Feature negotiation between the core and its clients.
//!
//! Spec rule 4 ("no fake features") is enforced here: this is the single list
//! a UI may gate its screens on, and it is derived from what the core actually
//! implements.

use serde::{Deserialize, Serialize};

use crate::pb;

/// Implementation status of a capability, mirroring
/// `gguf.v1.CapabilityStatus` so "unknown" and "planned" are first-class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureStatus {
    /// Implemented, tested and reachable from the API.
    Supported,
    /// Implemented but not yet validated for release (e.g. needs hardware).
    Experimental,
    /// Implemented by a future task.
    Planned,
    /// Cannot exist on this platform or with this build.
    Unsupported,
    /// Present but not validated on this machine.
    Unknown,
}

impl FeatureStatus {
    /// Wire representation.
    pub fn to_pb(self) -> pb::CapabilityStatus {
        match self {
            Self::Supported => pb::CapabilityStatus::Supported,
            Self::Experimental => pb::CapabilityStatus::Experimental,
            Self::Planned => pb::CapabilityStatus::Planned,
            Self::Unsupported => pb::CapabilityStatus::Unsupported,
            Self::Unknown => pb::CapabilityStatus::Unknown,
        }
    }

    /// Stable text form used by `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Experimental => "experimental",
            Self::Planned => "planned",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }
}

/// A named capability with its status and, when not supported, a reason the UI
/// can show verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feature {
    /// Stable identifier, e.g. `ipc`, `database`, `hardware_inventory`.
    pub id: String,
    /// Current status.
    pub status: FeatureStatus,
    /// Human readable explanation when `status` is not `Supported`.
    pub reason: Option<String>,
}

impl Feature {
    fn new(id: &str, status: FeatureStatus, reason: impl Into<Option<String>>) -> Self {
        Self {
            id: id.to_owned(),
            status,
            reason: reason.into(),
        }
    }

    /// Convert to the wire type.
    pub fn to_pb(&self) -> pb::Feature {
        pb::Feature {
            id: self.id.clone(),
            status: self.status.to_pb() as i32,
            reason: self.reason.clone().unwrap_or_default(),
        }
    }
}

/// The capability matrix of this build.
///
/// Order is stable and groups implemented features first so `gguf doctor` and
/// the UI render a predictable list.
pub fn features() -> Vec<Feature> {
    vec![
        Feature::new("ipc", FeatureStatus::Supported, None),
        Feature::new("event_stream", FeatureStatus::Supported, None),
        Feature::new("database", FeatureStatus::Supported, None),
        Feature::new("health", FeatureStatus::Supported, None),
        Feature::new("hardware_inventory", FeatureStatus::Supported, None),
        // Task 5: GGUF parsing, the SQLite registry and Hub downloads are real.
        Feature::new("model_registry", FeatureStatus::Supported, None),
        Feature::new("model_download", FeatureStatus::Supported, None),
        // Task 6: the llama.cpp adapter probes, loads and streams for real.
        Feature::new("runtime_llama_cpp", FeatureStatus::Supported, None),
        // Task 7: the planner, the lease ledger and the three-tier memory model.
        Feature::new("multi_gpu_scheduling", FeatureStatus::Supported, None),
        Feature::new("memory_paging", FeatureStatus::Supported, None),
        // Task 8: vision is real; the other modalities are listed by
        // gguf_engine_runtime_manager::modalities() with their own reasons.
        Feature::new("vision", FeatureStatus::Supported, None),
        // Task 8 follow-up: the non-LLM adapters exist. Whether each *works here*
        // depends on a runtime being installed, which `gguf modalities` reports
        // after probing - a capability being implemented is not the same claim as
        // a runtime being present, and the CLI shows the difference.
        Feature::new("image_generation", FeatureStatus::Supported, None),
        Feature::new("video_generation", FeatureStatus::Supported, None),
        Feature::new("speech_recognition", FeatureStatus::Supported, None),
        Feature::new("speech_synthesis", FeatureStatus::Supported, None),
        Feature::new(
            "music_generation",
            FeatureStatus::Planned,
            Some(
                "no music-generation runtime is integrated; it needs a new adapter and a \
                 manifest, not a flag"
                    .to_owned(),
            ),
        ),
        // Task 9 follow-up: InferenceService streams chat over the local transport,
        // which is the path the UI shells use.
        Feature::new("inference", FeatureStatus::Supported, None),
        // Follow-up after Task 9: the OpenAI-compatible gateway is real and
        // loopback-only by default.
        Feature::new("http_api", FeatureStatus::Supported, None),
    ]
}

/// The IPC endpoint name for a data directory.
///
/// Each data root gets its own endpoint, so two installations (or two test
/// suites) never collide, and so a stale socket from another user is never
/// reachable.
///
/// * Windows: `\\?\pipe\gguf-ai-engine-<hash>`
/// * Unix: `<data_root>/run/core.sock`
pub fn ipc_endpoint(data_root: &std::path::Path) -> String {
    if cfg!(windows) {
        format!("{}gguf-ai-engine-{}", PIPE_PREFIX, endpoint_tag(data_root))
    } else {
        data_root
            .join("run")
            .join("core.sock")
            .display()
            .to_string()
    }
}

/// Windows named-pipe prefix used by the core: the literal string `\\.\pipe\`.
///
/// Written with escapes rather than a raw string because a raw string cannot
/// end in a backslash, and an easy typo here produces a pipe name the OS simply
/// refuses to create.
pub const PIPE_PREFIX: &str = "\\\\.\\pipe\\";

/// Short stable tag for a data root (FNV-1a of the normalised path).
///
/// Deliberately short and hex: pipe names are limited to 256 characters and
/// long paths would otherwise overflow them.
fn endpoint_tag(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().to_lowercase();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn only_implemented_features_are_supported() {
        let all = features();
        let supported: Vec<&str> = all
            .iter()
            .filter(|f| f.status == FeatureStatus::Supported)
            .map(|f| f.id.as_str())
            .collect();
        assert_eq!(
            supported,
            vec![
                "ipc",
                "event_stream",
                "database",
                "health",
                "hardware_inventory",
                "model_registry",
                "model_download",
                "runtime_llama_cpp",
                "multi_gpu_scheduling",
                "memory_paging",
                "vision",
                "image_generation",
                "video_generation",
                "speech_recognition",
                "speech_synthesis",
                "inference",
                "http_api"
            ],
            "advertising a capability before it exists violates spec rule 4"
        );
    }

    #[test]
    fn every_non_supported_feature_explains_itself() {
        for feature in features() {
            if feature.status != FeatureStatus::Supported {
                assert!(
                    feature.reason.as_deref().is_some_and(|r| !r.is_empty()),
                    "{} has no reason",
                    feature.id
                );
            }
        }
    }

    #[test]
    fn feature_status_maps_to_the_wire_enum() {
        assert_eq!(
            FeatureStatus::Unsupported.to_pb(),
            pb::CapabilityStatus::Unsupported
        );
        assert_eq!(
            FeatureStatus::Planned.to_pb(),
            pb::CapabilityStatus::Planned
        );
        assert_eq!(
            FeatureStatus::Unknown.to_pb(),
            pb::CapabilityStatus::Unknown
        );
    }

    #[test]
    fn feature_round_trips_through_the_wire_type() {
        let all = features();
        let feature = all[0].clone();
        let wire = feature.to_pb();
        assert_eq!(wire.id, feature.id);
        assert_eq!(wire.status, pb::CapabilityStatus::Supported as i32);
    }

    #[test]
    fn endpoint_tag_matches_the_shared_test_vectors() {
        // These vectors are duplicated in the C# shell tests
        // (platforms/windows/GgufAiEngine.Shell.Core.Tests) so the two
        // implementations of "which pipe belongs to this data root" can never
        // drift apart silently.
        assert_eq!(endpoint_tag(Path::new("C:/data")), "cc34379d6c404275");
        assert_eq!(
            endpoint_tag(Path::new("F:/dsh-project/myModel/data")),
            "046164204fac7302"
        );
        assert_eq!(
            endpoint_tag(Path::new("/home/user/.local/share/gguf-ai-engine")),
            "8d31b980b94baba7"
        );
        // The tag is case-insensitive because Windows paths are.
        assert_eq!(
            endpoint_tag(Path::new("c:/DATA")),
            endpoint_tag(Path::new("C:/data"))
        );
    }

    #[test]
    fn endpoint_is_per_data_root_and_stable() {
        let a = ipc_endpoint(Path::new("/tmp/gguf-a"));
        let b = ipc_endpoint(Path::new("/tmp/gguf-b"));
        assert_ne!(a, b);
        assert_eq!(a, ipc_endpoint(Path::new("/tmp/gguf-a")));
        if cfg!(windows) {
            assert!(a.starts_with(PIPE_PREFIX));
            assert!(a.len() < 256);
        } else {
            assert!(a.ends_with("core.sock"));
        }
    }
}
