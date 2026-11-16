//! Speech synthesis through `audio.cpp` (GGUF weights).
//!
//! Piper stays available: it is tiny and instant, and its voice is ONNX. This is
//! the GGUF path - Qwen3 TTS CustomVoice - for users who want the whole stack on
//! GGUF and are willing to pay a few gigabytes for it.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::error::MultimodalError;
use crate::music::AudioBackend;

/// A GGUF speech-synthesis request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechRequest {
    /// Text to speak.
    pub text: String,
    /// Packaged voice id, e.g. `Vivian`.
    pub voice: String,
    /// Language hint, e.g. `en-us`; empty lets the model decide.
    pub language: String,
}

impl SpeechRequest {
    /// A request with the runtime's default voice.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            voice: "Vivian".to_owned(),
            language: String::new(),
        }
    }
}

/// Synthesise speech with `audio.cpp`, returning WAV bytes.
///
/// `model_dir` is the package directory (the runtime resolves the weights and
/// voice assets inside it).
pub fn synthesise(
    binary: &Path,
    model_dir: &Path,
    request: &SpeechRequest,
    backend: AudioBackend,
) -> Result<Vec<u8>, MultimodalError> {
    if request.text.trim().is_empty() {
        return Err(MultimodalError::Invalid(
            "the text must not be empty".to_owned(),
        ));
    }
    if !model_dir.is_dir() {
        return Err(MultimodalError::Invalid(format!(
            "the speech model directory was not found: {}",
            model_dir.display()
        )));
    }

    let out = temporary_wav();
    let mut command = std::process::Command::new(binary);
    command
        .arg("--task")
        .arg("tts")
        .arg("--family")
        .arg("qwen3_tts")
        .arg("--model")
        .arg(model_dir)
        .arg("--backend")
        .arg(backend.as_str())
        .arg("--text")
        .arg(&request.text)
        .arg("--request-option")
        .arg(format!("voice_id={}", request.voice))
        .arg("--out")
        .arg(&out)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());

    if !request.language.trim().is_empty() {
        command.arg("--language").arg(&request.language);
    }

    let output = command
        .output()
        .map_err(|err| MultimodalError::Runtime(format!("running audio.cpp: {err}")))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&out);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            &stdout
        } else {
            &stderr
        };
        return Err(MultimodalError::Runtime(format!(
            "audio.cpp could not synthesise speech: {}",
            detail.trim().rsplit('\n').next().unwrap_or("").trim()
        )));
    }

    let wav = std::fs::read(&out)
        .map_err(|err| MultimodalError::Runtime(format!("reading the audio: {err}")))?;
    let _ = std::fs::remove_file(&out);

    if wav.len() < 44 || &wav[0..4] != b"RIFF" {
        return Err(MultimodalError::Runtime(
            "the runtime did not produce a WAV file".to_owned(),
        ));
    }

    Ok(wav)
}

fn temporary_wav() -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("gguf-tts-{unique}.wav"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_refused_before_spawning() {
        let error = synthesise(
            Path::new("no-such-binary"),
            Path::new("no-such-dir"),
            &SpeechRequest::new("  "),
            AudioBackend::Cpu,
        )
        .unwrap_err();
        assert!(matches!(error, MultimodalError::Invalid(_)), "{error}");
    }

    #[test]
    fn a_missing_model_directory_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent");
        let error = synthesise(
            Path::new("no-such-binary"),
            &missing,
            &SpeechRequest::new("hello"),
            AudioBackend::Cpu,
        )
        .unwrap_err();
        assert!(error.to_string().contains("model directory"), "{error}");
    }

    #[test]
    fn the_default_voice_is_a_packaged_one() {
        assert_eq!(SpeechRequest::new("x").voice, "Vivian");
    }
}
