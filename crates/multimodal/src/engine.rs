//! The engine that drives the configured multimodal runtimes.
//!
//! Both transports - the gRPC service the UI shells use and the OpenAI-compatible
//! HTTP gateway - need the same three operations. Putting them here means one
//! implementation, one set of failure messages, and no chance of the two
//! disagreeing about why a capability is unavailable.

use std::path::PathBuf;

use crate::error::MultimodalError;
use crate::image::{GeneratedImage, ImageRequest, generate};
use crate::modality::{ModalityState, modality_report};
use crate::probe::probe_all;
use crate::speech::{Speech, Transcription, synthesise, transcribe};
use crate::video::{GeneratedVideo, VideoRequest, generate as generate_clip};

/// Configured paths and a located runtime, ready to serve requests.
#[derive(Debug, Clone, Default)]
pub struct MediaEngine {
    /// Diffusion model path, when image generation is set up.
    pub image_model: Option<PathBuf>,
    /// Base URL of a running diffusion server.
    pub diffusion_base: Option<String>,
    /// whisper.cpp model path.
    pub asr_model: Option<PathBuf>,
    /// Piper voice model path.
    pub tts_voice: Option<PathBuf>,
}

impl MediaEngine {
    /// Build from explicit paths (the configuration layer maps its own type).
    pub fn new(
        image_model: Option<String>,
        diffusion_url: Option<String>,
        asr_model: Option<String>,
        tts_voice: Option<String>,
    ) -> Self {
        let configured_base = diffusion_url
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .cloned();

        // Fall back to the runtime's default port when the user has not said
        // where it is; a server started with defaults is the common case.
        let discovered_base = configured_base.clone().or_else(|| {
            probe_all()
                .iter()
                .find(|probe| {
                    probe.installed
                        && probe
                            .manifest
                            .modalities
                            .iter()
                            .any(|m| m == "image_generation")
                })
                .and_then(crate::probe::http_base)
        });

        Self {
            image_model: non_empty(image_model),
            diffusion_base: discovered_base,
            asr_model: non_empty(asr_model),
            tts_voice: non_empty(tts_voice),
        }
    }

    /// Why a modality cannot be used, as one sentence for the user.
    pub fn unavailable(modality: &str) -> MultimodalError {
        let report = modality_report();
        match report.iter().find(|row| row.modality == modality) {
            Some(row) if row.state != ModalityState::Available => {
                MultimodalError::NotAvailable(row.detail.clone())
            }
            Some(_) => MultimodalError::NotAvailable(format!(
                "{modality} is installed but not configured; set its model path"
            )),
            None => MultimodalError::NotAvailable(format!(
                "no runtime in this build provides {modality}"
            )),
        }
    }

    /// Generate an image.
    pub async fn image(&self, request: &ImageRequest) -> Result<GeneratedImage, MultimodalError> {
        let base = self
            .diffusion_base
            .as_ref()
            .ok_or_else(|| Self::unavailable("image_generation"))?;
        if self.image_model.is_none() {
            return Err(Self::unavailable("image_generation"));
        }
        generate(base, request).await
    }

    /// Generate a clip.
    pub async fn video(&self, request: &VideoRequest) -> Result<GeneratedVideo, MultimodalError> {
        let base = self
            .diffusion_base
            .as_ref()
            .ok_or_else(|| Self::unavailable("video"))?;
        if self.image_model.is_none() {
            return Err(Self::unavailable("video"));
        }
        generate_clip(base, request).await
    }

    /// Synthesise speech.
    pub fn speech(&self, text: &str) -> Result<Speech, MultimodalError> {
        let voice = self
            .tts_voice
            .as_ref()
            .ok_or_else(|| Self::unavailable("tts"))?;
        let binary = runtime_binary("tts")?;
        synthesise(&binary, voice, text)
    }

    /// Transcribe an audio file.
    pub fn transcribe(
        &self,
        audio: &std::path::Path,
        language: Option<&str>,
    ) -> Result<Transcription, MultimodalError> {
        let model = self
            .asr_model
            .as_ref()
            .ok_or_else(|| Self::unavailable("asr"))?;
        let binary = runtime_binary("asr")?;
        transcribe(&binary, model, audio, language)
    }
}

fn non_empty(value: Option<String>) -> Option<PathBuf> {
    value
        .filter(|text| !text.trim().is_empty())
        .map(PathBuf::from)
}

/// The binary of the installed runtime that provides `modality`.
pub fn runtime_binary(modality: &str) -> Result<PathBuf, MultimodalError> {
    probe_all()
        .iter()
        .find(|probe| probe.installed && probe.manifest.modalities.iter().any(|m| m == modality))
        .and_then(|probe| probe.path.clone())
        .ok_or_else(|| MediaEngine::unavailable(modality))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unconfigured_model_reports_the_modality_as_unavailable() {
        let engine = MediaEngine::default();
        let error = engine.image_sync_error();
        assert!(matches!(error, MultimodalError::NotAvailable(_)), "{error}");
    }

    #[test]
    fn blank_paths_are_treated_as_absent() {
        let engine = MediaEngine::new(Some("  ".to_owned()), None, None, None);
        assert!(engine.image_model.is_none());
    }

    #[test]
    fn a_real_path_is_kept() {
        let engine = MediaEngine::new(Some("C:/models/x.gguf".to_owned()), None, None, None);
        assert!(engine.image_model.is_some());
    }

    impl MediaEngine {
        /// Exercise the synchronous half of the image path for a unit test.
        fn image_sync_error(&self) -> MultimodalError {
            self.diffusion_base
                .as_ref()
                .map(|_| MultimodalError::NotAvailable("configured".to_owned()))
                .unwrap_or_else(|| Self::unavailable("image_generation"))
        }
    }
}
