//! Media endpoints: images, speech synthesis and transcription (spec §9.2).
//!
//! These call non-LLM runtimes, so they are gated the same way the LLM path is:
//! a request is only attempted when a runtime that provides the modality was
//! found by probing. When none is, the response is 503 with the reason the probe
//! produced - not a generic "internal error", and never a fake success.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::error::ApiError;
use crate::openai::{
    AudioSpeechRequest, ImageDatum, ImageGenerationRequest, ImageGenerationResponse,
};

/// What the gateway needs from the media runtimes.
#[async_trait]
pub trait MediaBackend: Send + Sync {
    /// Generate an image, returning PNG bytes.
    async fn image(&self, request: &ImageGenerationRequest) -> Result<Vec<u8>, ApiError>;

    /// Synthesise speech, returning WAV bytes.
    async fn speech(&self, request: &AudioSpeechRequest) -> Result<Vec<u8>, ApiError>;

    /// Transcribe an audio file, returning the text.
    async fn transcribe(
        &self,
        audio: Vec<u8>,
        filename: &str,
        language: Option<String>,
    ) -> Result<String, ApiError>;
}

/// The media backend backed by the local runtimes.
#[derive(Debug, Clone, Default)]
pub struct LocalMedia {
    /// Diffusion model for image generation.
    pub image_model: Option<PathBuf>,
    /// whisper.cpp model.
    pub asr_model: Option<PathBuf>,
    /// Piper voice model.
    pub tts_voice: Option<PathBuf>,
    /// Base URL of a running diffusion server, when one is configured.
    pub diffusion_base: Option<String>,
}

impl LocalMedia {
    /// Build from the engine configuration.
    pub fn from_config(config: &gguf_engine_config::MultimodalConfig) -> Self {
        let image_model = config
            .image_model
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);

        // The diffusion server is the runtime's own process; the engine does not
        // start it here, because it needs the model on its command line and the
        // user may already be running it.
        let configured_url = config
            .diffusion_url
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .cloned();

        let diffusion_base = configured_url.or_else(|| {
            gguf_engine_multimodal::probe_all()
                .iter()
                .find(|probe| {
                    probe.installed
                        && probe
                            .manifest
                            .modalities
                            .iter()
                            .any(|m| m == "image_generation")
                })
                .and_then(gguf_engine_multimodal::http_base)
        });

        Self {
            image_model,
            asr_model: config
                .asr_model
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from),
            tts_voice: config
                .tts_voice
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from),
            diffusion_base,
        }
    }
}

#[async_trait]
impl MediaBackend for LocalMedia {
    async fn image(&self, request: &ImageGenerationRequest) -> Result<Vec<u8>, ApiError> {
        let base = self.diffusion_base.as_ref().ok_or_else(|| {
            ApiError::NotAvailable(
                gguf_engine_multimodal::modality_report()
                    .into_iter()
                    .find(|row| row.modality == "image_generation")
                    .map(|row| row.detail)
                    .unwrap_or_else(|| "no diffusion runtime is installed".to_owned()),
            )
        })?;

        if self.image_model.is_none() {
            return Err(ApiError::NotAvailable(
                "set multimodal.image_model in the configuration".to_owned(),
            ));
        }

        let (width, height) = parse_size(request.size.as_deref().unwrap_or("512x512"));

        let mut image = gguf_engine_multimodal::ImageRequest::new(&request.prompt);
        image.width = width;
        image.height = height;
        image.steps = 20;

        let generated = gguf_engine_multimodal::generate_image(base, &image)
            .await
            .map_err(|err| ApiError::Backend(err.to_string()))?;

        Ok(generated.bytes)
    }

    async fn speech(&self, request: &AudioSpeechRequest) -> Result<Vec<u8>, ApiError> {
        let voice = self.tts_voice.as_ref().ok_or_else(|| {
            ApiError::NotAvailable("set multimodal.tts_voice in the configuration".to_owned())
        })?;

        let probe = gguf_engine_multimodal::probe_all()
            .iter()
            .find(|probe| probe.installed && probe.manifest.modalities.iter().any(|m| m == "tts"))
            .ok_or_else(|| {
                ApiError::NotAvailable("no speech-synthesis runtime is installed".to_owned())
            })?;

        let binary = probe
            .path
            .clone()
            .ok_or_else(|| ApiError::Backend("the TTS runtime has no path".to_owned()))?;

        let speech = gguf_engine_multimodal::synthesise(&binary, voice, &request.input)
            .map_err(|err| ApiError::Backend(err.to_string()))?;

        Ok(speech.wav)
    }

    async fn transcribe(
        &self,
        audio: Vec<u8>,
        filename: &str,
        language: Option<String>,
    ) -> Result<String, ApiError> {
        let model = self.asr_model.as_ref().ok_or_else(|| {
            ApiError::NotAvailable("set multimodal.asr_model in the configuration".to_owned())
        })?;

        let probe = gguf_engine_multimodal::probe_all()
            .iter()
            .find(|probe| probe.installed && probe.manifest.modalities.iter().any(|m| m == "asr"))
            .ok_or_else(|| {
                ApiError::NotAvailable("no speech-recognition runtime is installed".to_owned())
            })?;

        let binary = probe
            .path
            .clone()
            .ok_or_else(|| ApiError::Backend("the ASR runtime has no path".to_owned()))?;

        // whisper.cpp reads from a file, so the upload is written to a temporary
        // one and removed afterwards. The extension is preserved because the
        // tool uses it to decide the container.
        let extension = std::path::Path::new(filename)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("wav");
        let path = std::env::temp_dir().join(format!(
            "gguf-upload-{}.{extension}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        tokio::fs::write(&path, &audio)
            .await
            .map_err(|err| ApiError::Backend(format!("storing the upload: {err}")))?;

        let result = tokio::task::spawn_blocking({
            let binary = binary.clone();
            let model = model.clone();
            let path = path.clone();
            move || gguf_engine_multimodal::transcribe(&binary, &model, &path, language.as_deref())
        })
        .await
        .map_err(|err| ApiError::Backend(format!("the transcription task failed: {err}")))?;

        let _ = tokio::fs::remove_file(&path).await;

        let transcription = result.map_err(|err| ApiError::Backend(err.to_string()))?;
        Ok(transcription.text)
    }
}

/// Parse an OpenAI `size` value (`512x512`, `1024x768`) into width and height.
///
/// Anything unparseable falls back to 512x512 rather than failing: the size is a
/// hint, and refusing a request over it would be worse than drawing a square.
pub fn parse_size(size: &str) -> (u32, u32) {
    let mut parts = size.split(['x', 'X']);
    let width = parts.next().and_then(|v| v.trim().parse::<u32>().ok());
    let height = parts.next().and_then(|v| v.trim().parse::<u32>().ok());

    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => (width, height),
        _ => (512, 512),
    }
}

/// Build the response the gateway returns for a generated image.
pub fn image_response(bytes: Vec<u8>, model: String) -> ImageGenerationResponse {
    use base64::Engine as _;
    ImageGenerationResponse {
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default(),
        data: vec![ImageDatum {
            b64_json: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            url: None,
        }],
        model,
    }
}

/// Helper for tests and callers that need an `Arc`.
pub fn shared(media: LocalMedia) -> Arc<dyn MediaBackend> {
    Arc::new(media)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_parsed_in_both_orders() {
        assert_eq!(parse_size("512x512"), (512, 512));
        assert_eq!(parse_size("1024x768"), (1024, 768));
        assert_eq!(parse_size("768X1024"), (768, 1024));
    }

    #[test]
    fn an_unparseable_size_falls_back_rather_than_failing() {
        assert_eq!(parse_size("huge"), (512, 512));
        assert_eq!(parse_size(""), (512, 512));
        assert_eq!(parse_size("0x0"), (512, 512));
        assert_eq!(parse_size("512"), (512, 512));
    }

    #[test]
    fn an_image_response_carries_base64_bytes() {
        let response = image_response(vec![1, 2, 3], "m".to_owned());
        assert_eq!(response.data.len(), 1);
        assert!(response.data[0].b64_json.is_some());
        assert!(response.data[0].url.is_none());
    }
}
