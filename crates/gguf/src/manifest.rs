//! Model manifests and compatibility reports (spec §7.2, §7.4).
//!
//! A manifest is what the product knows about a model *after* looking at the
//! bytes and the repository metadata — never what the file name suggests. The
//! rule from spec §7.2 is enforced by construction: every field is derived from
//! either the GGUF header or a caller-supplied repository fact, and a field that
//! could only be guessed stays `None`.

use serde::{Deserialize, Serialize};

use crate::parser::Gguf;

/// Manifest schema version.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// What a model can consume or produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    /// Text in, text out.
    Text,
    /// Images in.
    Vision,
    /// Audio in.
    Audio,
    /// Embeddings.
    Embedding,
}

/// A file that belongs to a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFile {
    /// Path inside the repository.
    pub path: String,
    /// Role: `weights`, `mmproj`, `tokenizer`, ...
    pub role: String,
    /// Whether the model cannot run without it.
    pub required: bool,
    /// Size in bytes, from repository metadata or the local file.
    pub size_bytes: u64,
    /// SHA-256 in lowercase hex, when known.
    pub sha256: Option<String>,
}

/// What a runtime must provide to load this model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRequirement {
    /// e.g. `llama.cpp`.
    pub runtime: String,
    /// Architectures that runtime must support.
    pub architectures: Vec<String>,
    /// Why the requirement exists.
    pub reason: String,
}

/// A model manifest (spec §7.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifest {
    /// Schema version of this document.
    pub schema_version: u32,
    /// `publisher/name`.
    pub model_id: String,
    /// Immutable commit or resolved revision.
    pub revision: String,
    /// Architecture from the GGUF header, when known.
    pub architecture: Option<String>,
    /// Declared modalities.
    pub modalities: Vec<Modality>,
    /// Container format, always `gguf` today.
    pub format: String,
    /// Files, in repository order.
    pub files: Vec<ModelFile>,
    /// Runtime requirements.
    pub runtime_requirements: Vec<RuntimeRequirement>,
    /// Extra components (projectors, vocoders) the model needs.
    pub components: Vec<String>,
    /// License identifier, only when the repository states one.
    pub license: Option<String>,
}

impl ModelManifest {
    /// Total size of all files.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size_bytes).sum()
    }

    /// True when a required file has no hash, so it cannot be verified.
    pub fn has_unverifiable_files(&self) -> bool {
        self.files
            .iter()
            .any(|file| file.required && file.sha256.is_none())
    }
}

/// Compatibility state (spec §7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityState {
    /// Verified against a runtime and this hardware.
    Supported,
    /// Structure understood, but not yet run.
    CompatibleUnverified,
    /// A required component is missing.
    MissingComponents,
    /// No runtime knows this architecture.
    UnsupportedArchitecture,
    /// The hardware cannot provide the backends the runtime needs.
    UnsupportedBackend,
    /// Not enough memory or storage.
    InsufficientResources,
    /// Cannot be determined.
    Unknown,
}

impl CompatibilityState {
    /// Stable text form for the UI and the CLI.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::CompatibleUnverified => "compatible_unverified",
            Self::MissingComponents => "missing_components",
            Self::UnsupportedArchitecture => "unsupported_architecture",
            Self::UnsupportedBackend => "unsupported_backend",
            Self::InsufficientResources => "insufficient_resources",
            Self::Unknown => "unknown",
        }
    }
}

/// A compatibility verdict with its reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityReport {
    /// The verdict.
    pub state: CompatibilityState,
    /// Why, in words a user can act on.
    pub reason: String,
}

/// Architectures this build knows llama.cpp supports.
///
/// Kept intentionally small and explicit: adding a name here is a claim that the
/// runtime can load it. Everything else becomes `UnsupportedArchitecture` with an
/// explanation rather than a hopeful `compatible_unverified`.
pub const KNOWN_ARCHITECTURES: &[&str] = &[
    "llama",
    "mistral",
    "qwen2",
    "qwen2moe",
    "qwen3",
    "gemma",
    "gemma2",
    "gemma3",
    "phi2",
    "phi3",
    "phi4",
    "deepseek2",
    "deepseek41",
    "falcon",
    "mpt",
    "gpt2",
    "starcoder2",
    "command-r",
    "internlm2",
    "minicpm",
    "yi",
    "cohere2",
    "olmo2",
    "glm4",
    "granite",
    "exaone",
];

/// Metadata keys that indicate a multimodal projector rather than a text model.
const VISION_KEYS: &[&str] = &[
    "clip.has_vision_encoder",
    "clip.has_audio_encoder",
    "clip.vision.image_size",
];

/// Derive a manifest from a parsed GGUF file and repository facts.
///
/// `model_id` and `revision` come from the caller because they are repository
/// facts, not file contents — guessing them from the file name is exactly what
/// spec §7.2 forbids.
pub fn manifest_from_gguf(
    gguf: &Gguf,
    model_id: &str,
    revision: &str,
    file_name: &str,
    sha256: Option<String>,
    license: Option<String>,
) -> ModelManifest {
    let architecture = gguf.architecture().map(str::to_owned);

    let mut modalities = vec![Modality::Text];
    let mut components = Vec::new();
    for key in VISION_KEYS {
        if gguf.metadata.contains_key(*key) {
            modalities.push(Modality::Vision);
            components.push("vision encoder".to_owned());
            break;
        }
    }

    let runtime_requirements = match &architecture {
        Some(arch) => vec![RuntimeRequirement {
            runtime: "llama.cpp".to_owned(),
            architectures: vec![arch.clone()],
            reason: format!("{arch} is a llama.cpp model architecture"),
        }],
        None => Vec::new(),
    };

    ModelManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        model_id: model_id.to_owned(),
        revision: revision.to_owned(),
        architecture,
        modalities,
        format: "gguf".to_owned(),
        files: vec![ModelFile {
            path: file_name.to_owned(),
            role: "weights".to_owned(),
            required: true,
            size_bytes: gguf.file_len,
            sha256,
        }],
        runtime_requirements,
        components,
        license,
    }
}

/// Evaluate a manifest against what this build can actually do.
///
/// Today no runtime adapter is installed (Task 6), so a structurally valid model
/// can only reach `CompatibleUnverified`. Reporting `Supported` would be a lie,
/// and the reason string says so explicitly.
pub fn compatibility(manifest: &ModelManifest, runtime_installed: bool) -> CompatibilityReport {
    let Some(architecture) = manifest.architecture.as_deref() else {
        return CompatibilityReport {
            state: CompatibilityState::Unknown,
            reason: "the file does not declare general.architecture".to_owned(),
        };
    };

    if !KNOWN_ARCHITECTURES.contains(&architecture) {
        return CompatibilityReport {
            state: CompatibilityState::UnsupportedArchitecture,
            reason: format!(
                "{architecture} is not in this build's known llama.cpp architecture list"
            ),
        };
    }

    // Download integrity is the downloader's job (see
    // gguf-engine-model-hub), not a property of the model architecture. A local
    // file with no published hash still has an architecture we can judge, so the
    // verdict below must not depend on it.
    let _unverifiable = manifest.has_unverifiable_files();

    if !runtime_installed {
        return CompatibilityReport {
            state: CompatibilityState::CompatibleUnverified,
            reason: format!(
                "{architecture} is supported by llama.cpp, but no runtime adapter is installed yet (Task 6)"
            ),
        };
    }

    CompatibilityReport {
        state: CompatibilityState::CompatibleUnverified,
        reason: format!(
            "{architecture}: a runtime is installed but this model has not been loaded yet"
        ),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(architecture: Option<&str>, sha: Option<&str>) -> ModelManifest {
        ModelManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            model_id: "publisher/model".into(),
            revision: "commit".into(),
            architecture: architecture.map(str::to_owned),
            modalities: vec![Modality::Text],
            format: "gguf".into(),
            files: vec![ModelFile {
                path: "model.gguf".into(),
                role: "weights".into(),
                required: true,
                size_bytes: 1024,
                sha256: sha.map(str::to_owned),
            }],
            runtime_requirements: Vec::new(),
            components: Vec::new(),
            license: None,
        }
    }

    #[test]
    fn a_known_architecture_without_a_runtime_is_compatible_unverified() {
        let report = compatibility(&manifest(Some("llama"), None), false);
        assert_eq!(report.state, CompatibilityState::CompatibleUnverified);
        assert!(
            report.reason.contains("no runtime adapter"),
            "{}",
            report.reason
        );
    }

    #[test]
    fn a_known_architecture_with_a_runtime_is_still_unverified_until_loaded() {
        let report = compatibility(&manifest(Some("qwen2"), Some("a")), true);
        assert_eq!(report.state, CompatibilityState::CompatibleUnverified);
        assert!(
            report.reason.contains("has not been loaded"),
            "{}",
            report.reason
        );
    }

    #[test]
    fn an_unknown_architecture_is_unsupported() {
        let report = compatibility(&manifest(Some("made_up_arch"), None), true);
        assert_eq!(report.state, CompatibilityState::UnsupportedArchitecture);
        assert!(report.reason.contains("made_up_arch"), "{}", report.reason);
    }

    #[test]
    fn a_missing_architecture_is_unknown() {
        let report = compatibility(&manifest(None, None), false);
        assert_eq!(report.state, CompatibilityState::Unknown);
    }

    #[test]
    fn download_integrity_does_not_change_the_compatibility_verdict() {
        // A local file with no published hash still has a judgeable architecture.
        let no_hash = compatibility(&manifest(Some("llama"), None), false);
        let with_hash = compatibility(&manifest(Some("llama"), Some("a")), false);
        assert_eq!(no_hash.state, with_hash.state);
    }
}
