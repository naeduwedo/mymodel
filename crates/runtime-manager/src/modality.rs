//! Modality registry (spec 4.6, Task 8).
//!
//! Spec rule 4 forbids showing a capability that does not exist. The registry is
//! the single place that answers "can this build do image / video / audio /
//! music?", and it answers with a status and a reason, never a boolean.
//!
//! Each entry was decided by actually trying it, or by establishing that no
//! runtime exists on this machine.

use serde::{Deserialize, Serialize};

/// A modality the product may expose.
///
/// The variants beyond `Text` and `Vision` are the shared vocabulary used by the
/// multimodal crate's manifests; they are not returned by `modalities()` because
/// this crate does not implement them.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    /// Text in, text out.
    Text,
    /// Image in, text out (vision).
    Vision,
    /// Text in, image out (diffusion).
    ImageGeneration,
    /// Video in or out.
    Video,
    /// Speech recognition.
    Asr,
    /// Speech synthesis.
    Tts,
    /// Music generation.
    Music,
}

impl Modality {
    /// Stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Vision => "vision",
            Self::ImageGeneration => "image_generation",
            Self::Video => "video",
            Self::Asr => "asr",
            Self::Tts => "tts",
            Self::Music => "music",
        }
    }
}

/// Implementation status of one modality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModalityStatus {
    /// Implemented and verified on real hardware.
    Supported,
    /// Implemented but not yet validated end to end.
    Experimental,
    /// No adapter exists in this build.
    Unsupported,
}

impl ModalityStatus {
    /// Stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Experimental => "experimental",
            Self::Unsupported => "unsupported",
        }
    }
}

/// One modality and what this build can do with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalitySupport {
    /// Which modality.
    pub modality: Modality,
    /// Status.
    pub status: ModalityStatus,
    /// Runtime that implements it, when one does.
    pub runtime: Option<String>,
    /// Explanation the UI shows verbatim.
    pub detail: String,
}

/// The modalities **this crate's adapters** implement.
///
/// Only text and vision: everything else is provided by a separate upstream
/// runtime with its own binary and protocol, and is reported by
/// `gguf_engine_multimodal::modality_report` after actually probing for it.
///
/// The split is deliberate. Listing `image_generation` here as "unsupported"
/// would be a guess: whether an image can be made depends on whether
/// stable-diffusion.cpp is installed, which this crate cannot know.
pub fn modalities() -> Vec<ModalitySupport> {
    vec![
        ModalitySupport {
            modality: Modality::Text,
            status: ModalityStatus::Supported,
            runtime: Some("llama.cpp".to_owned()),
            detail: "streaming chat through llama-server".to_owned(),
        },
        ModalitySupport {
            modality: Modality::Vision,
            status: ModalityStatus::Supported,
            runtime: Some("llama.cpp mtmd".to_owned()),
            detail: "image in, text out; needs an mmproj companion file".to_owned(),
        },
    ]
}

/// True when the build can handle a modality at all.
pub fn supports(modality: Modality) -> bool {
    modalities()
        .iter()
        .any(|entry| entry.modality == modality && entry.status == ModalityStatus::Supported)
}
