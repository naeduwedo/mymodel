//! Runtime manifests for the non-LLM modalities (spec §2.3).
//!
//! The spec requires every runtime to declare, in its own manifest, which
//! operating systems, CPU architectures, GPU backends, model architectures,
//! quantizations, parallel modes, offload/paging support and modalities it
//! provides. This module is that declaration.
//!
//! It exists so the product never has to guess. A modality is offered because a
//! runtime that claims it is installed and answered a probe, not because a
//! feature list was written by hand.

use serde::{Deserialize, Serialize};

/// What a runtime does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Diffusion image generation (and, for some, video).
    Diffusion,
    /// Speech recognition.
    SpeechRecognition,
    /// Speech synthesis.
    SpeechSynthesis,
    /// Music or general audio generation.
    MusicGeneration,
}

impl RuntimeKind {
    /// Stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Diffusion => "diffusion",
            Self::SpeechRecognition => "speech_recognition",
            Self::SpeechSynthesis => "speech_synthesis",
            Self::MusicGeneration => "music_generation",
        }
    }
}

/// How the engine talks to the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Invocation {
    /// An HTTP server: the engine starts it and speaks its API.
    Http {
        /// Executable name, e.g. `sd`.
        binary: String,
        /// Default port.
        default_port: u16,
    },
    /// A command-line tool invoked once per request.
    Cli {
        /// Executable name.
        binary: String,
    },
}

/// A runtime's complete capability declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeManifest {
    /// Stable id.
    pub id: String,
    /// Name shown to the user.
    pub display_name: String,
    /// What it does.
    pub kind: RuntimeKind,
    /// Operating systems it runs on, as `std::env::consts::OS` values.
    pub os: Vec<String>,
    /// CPU architectures it runs on, as `std::env::consts::ARCH` values.
    pub archs: Vec<String>,
    /// GPU backends it can use; `cpu` is always present.
    pub backends: Vec<String>,
    /// Model architectures it can load.
    pub model_archs: Vec<String>,
    /// Quantizations it can load.
    pub quantizations: Vec<String>,
    /// Parallel modes it supports.
    pub parallel_modes: Vec<String>,
    /// Whether layers can be offloaded to a GPU.
    pub gpu_offload: bool,
    /// Whether it pages model data itself.
    pub paging: bool,
    /// Modalities it provides.
    pub modalities: Vec<String>,
    /// How to invoke it.
    pub invocation: Invocation,
}

impl RuntimeManifest {
    /// Whether this manifest claims to run on the current machine.
    pub fn runs_here(&self) -> bool {
        self.os.iter().any(|os| os == std::env::consts::OS)
            && self.archs.iter().any(|arch| arch == std::env::consts::ARCH)
    }

    /// The executable this runtime is invoked through.
    pub fn binary(&self) -> &str {
        match &self.invocation {
            Invocation::Http { binary, .. } | Invocation::Cli { binary } => binary,
        }
    }
}

/// The runtimes this build knows how to drive.
///
/// Each entry is a real upstream project with a real protocol. Nothing here is
/// aspirational: a manifest with no adapter behind it would be a fake feature.
pub fn builtin_manifests() -> Vec<RuntimeManifest> {
    vec![
        RuntimeManifest {
            id: "stable-diffusion.cpp".to_owned(),
            display_name: "stable-diffusion.cpp".to_owned(),
            kind: RuntimeKind::Diffusion,
            os: vec!["windows".to_owned(), "linux".to_owned(), "macos".to_owned()],
            archs: vec!["x86_64".to_owned(), "aarch64".to_owned()],
            backends: vec![
                "cpu".to_owned(),
                "cuda".to_owned(),
                "vulkan".to_owned(),
                "metal".to_owned(),
            ],
            model_archs: vec![
                "sd1".to_owned(),
                "sd2".to_owned(),
                "sdxl".to_owned(),
                "flux".to_owned(),
                "sd3".to_owned(),
                "wan".to_owned(),
            ],
            quantizations: vec!["f16".to_owned(), "q8_0".to_owned(), "q4_k".to_owned()],
            parallel_modes: vec!["none".to_owned()],
            gpu_offload: true,
            // It keeps the model resident and manages its own memory, like
            // llama.cpp; the engine must not move tensors into it (ADR-005).
            paging: false,
            modalities: vec!["image_generation".to_owned(), "video".to_owned()],
            // The server binary, not the one-shot CLI: a diffusion model takes
            // tens of seconds to load, so reloading per request would dominate
            // the cost of a small image.
            invocation: Invocation::Http {
                binary: "sd-server".to_owned(),
                default_port: 1234,
            },
        },
        RuntimeManifest {
            id: "whisper.cpp".to_owned(),
            display_name: "whisper.cpp".to_owned(),
            kind: RuntimeKind::SpeechRecognition,
            os: vec!["windows".to_owned(), "linux".to_owned(), "macos".to_owned()],
            archs: vec!["x86_64".to_owned(), "aarch64".to_owned()],
            backends: vec!["cpu".to_owned(), "cuda".to_owned(), "vulkan".to_owned()],
            model_archs: vec!["whisper".to_owned()],
            quantizations: vec!["f16".to_owned(), "q5_0".to_owned(), "q8_0".to_owned()],
            parallel_modes: vec!["none".to_owned()],
            gpu_offload: true,
            paging: false,
            modalities: vec!["asr".to_owned()],
            invocation: Invocation::Cli {
                binary: "whisper-cli".to_owned(),
            },
        },
        RuntimeManifest {
            // A ggml runtime for audio models, the same family as llama.cpp: it
            // loads GGUF weights and runs them itself, with no Python involved.
            id: "audio.cpp".to_owned(),
            display_name: "AudioCPP".to_owned(),
            kind: RuntimeKind::MusicGeneration,
            os: vec!["windows".to_owned(), "linux".to_owned(), "macos".to_owned()],
            archs: vec!["x86_64".to_owned(), "aarch64".to_owned()],
            backends: vec![
                "cpu".to_owned(),
                "cuda".to_owned(),
                "vulkan".to_owned(),
                "metal".to_owned(),
            ],
            model_archs: vec![
                "stable_audio".to_owned(),
                "ace_step".to_owned(),
                "heartmula".to_owned(),
                "kokoro".to_owned(),
                "qwen3_tts".to_owned(),
            ],
            quantizations: vec!["f16".to_owned(), "bf16".to_owned(), "q8_0".to_owned()],
            parallel_modes: vec!["none".to_owned()],
            gpu_offload: true,
            paging: false,
            // Music and sound effects. Speech is covered by Piper here; this
            // runtime can also do TTS, but claiming it without a verified voice
            // package would be a promise the build has not kept.
            modalities: vec!["music".to_owned(), "sfx".to_owned()],
            invocation: Invocation::Cli {
                binary: "audiocpp_cli".to_owned(),
            },
        },
        RuntimeManifest {
            id: "piper".to_owned(),
            display_name: "Piper".to_owned(),
            kind: RuntimeKind::SpeechSynthesis,
            os: vec!["windows".to_owned(), "linux".to_owned(), "macos".to_owned()],
            archs: vec!["x86_64".to_owned(), "aarch64".to_owned()],
            backends: vec!["cpu".to_owned()],
            model_archs: vec!["vits".to_owned()],
            quantizations: vec!["onnx".to_owned()],
            parallel_modes: vec!["none".to_owned()],
            // Piper is CPU-only by design: a voice model is small enough that
            // a GPU would add latency, not remove it.
            gpu_offload: false,
            paging: false,
            modalities: vec!["tts".to_owned()],
            invocation: Invocation::Cli {
                binary: "piper".to_owned(),
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_manifest_declares_a_binary_and_at_least_one_modality() {
        for manifest in builtin_manifests() {
            assert!(
                !manifest.binary().is_empty(),
                "{} has no binary",
                manifest.id
            );
            assert!(
                !manifest.modalities.is_empty(),
                "{} declares no modality",
                manifest.id
            );
            assert!(
                manifest.backends.iter().any(|b| b == "cpu"),
                "{} must be able to fall back to the CPU",
                manifest.id
            );
        }
    }

    #[test]
    fn the_known_runtimes_cover_the_four_modalities_the_spec_asks_for() {
        // Bind the list first: a borrow of a temporary cannot outlive the call.
        let manifests = builtin_manifests();
        let modalities: Vec<&str> = manifests
            .iter()
            .flat_map(|m| m.modalities.iter().map(String::as_str))
            .collect();

        // Every modality the product offers is claimed by some manifest; nothing
        // is advertised without a runtime behind it.
        for expected in ["image_generation", "video", "asr", "tts", "music"] {
            assert!(
                modalities.contains(&expected),
                "no runtime provides {expected}"
            );
        }
    }

    #[test]
    fn a_manifest_needs_a_current_os_and_arch_to_run_here() {
        let mut manifests = builtin_manifests();
        let mut manifest = manifests.remove(0);
        assert!(manifest.runs_here(), "the defaults target this machine");

        manifest.os = vec!["plan9".to_owned()];
        assert!(!manifest.runs_here());

        manifest.os = vec![std::env::consts::OS.to_owned()];
        manifest.archs = vec!["pdp11".to_owned()];
        assert!(!manifest.runs_here());
    }
}
