//! Music and sound-effect generation through `audio.cpp`.
//!
//! `audio.cpp` is a ggml runtime like llama.cpp: it loads GGUF weights, has no
//! Python dependency, and runs the model itself. It is a command-line tool, not
//! a server, so one invocation produces one clip - which matches how a music
//! request works: nobody wants two songs from one prompt.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::error::MultimodalError;

/// A music or sound-effect request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MusicRequest {
    /// What to generate, in English (the model's prompt language).
    pub prompt: String,
    /// Length in seconds.
    pub duration_seconds: u32,
}

impl MusicRequest {
    /// A request with defaults that produce a short, quick clip.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            duration_seconds: 10,
        }
    }
}

/// A generated clip of audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedAudio {
    /// WAV bytes.
    pub wav: Vec<u8>,
    /// Sample rate the runtime reported.
    pub sample_rate: u32,
    /// Channel count the runtime reported.
    pub channels: u32,
}

/// Which compute backend the runtime should use.
///
/// Measured on this machine with Stable Audio 3 Small Music:
///
/// | Weights | Backend | Result |
/// | --- | --- | --- |
/// | q8_0 | cuda | **aborts** - ggml's MMQ kernel asserts (`mmq_x_best=0`) |
/// | q8_0 | cpu | works |
/// | f16 | cuda | works |
///
/// So the pair matters, not the backend alone. CUDA is the default because it is
/// the faster path, and the configuration names an f16 package to match; a Q8_0
/// package on CUDA fails loudly rather than producing bad audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioBackend {
    /// CPU only. Works with every quantization.
    Cpu,
    /// CUDA. Faster, but needs weights the kernels support.
    #[default]
    Cuda,
}

impl AudioBackend {
    /// The runtime's own name for this backend.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
        }
    }
}

/// Generate music with `audio.cpp`.
///
/// `model_dir` is the package directory, not the `.gguf` file: the runtime
/// expects a layout it can resolve, and passing a file path makes it fail with a
/// message about a missing component.
pub fn generate(
    binary: &Path,
    model_dir: &Path,
    request: &MusicRequest,
    backend: AudioBackend,
) -> Result<GeneratedAudio, MultimodalError> {
    if request.prompt.trim().is_empty() {
        return Err(MultimodalError::Invalid(
            "the prompt must not be empty".to_owned(),
        ));
    }
    if request.duration_seconds == 0 || request.duration_seconds > 600 {
        return Err(MultimodalError::Invalid(
            "duration must be between 1 and 600 seconds".to_owned(),
        ));
    }
    if !model_dir.is_dir() {
        return Err(MultimodalError::Invalid(format!(
            "the music model directory was not found: {}",
            model_dir.display()
        )));
    }

    let out = temporary_wav();
    let output = std::process::Command::new(binary)
        .arg("--task")
        .arg("gen")
        .arg("--family")
        .arg("stable_audio")
        .arg("--model")
        .arg(model_dir)
        .arg("--backend")
        .arg(backend.as_str())
        .arg("--text")
        .arg(&request.prompt)
        .arg("--duration-seconds")
        .arg(request.duration_seconds.to_string())
        .arg("--out")
        .arg(&out)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .map_err(|err| MultimodalError::Runtime(format!("running audio.cpp: {err}")))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&out);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(MultimodalError::Runtime(format!(
            "audio.cpp failed ({}): {}",
            output
                .status
                .code()
                .map_or("no code".to_owned(), |c| c.to_string()),
            tail(
                if stderr.trim().is_empty() {
                    &stdout
                } else {
                    &stderr
                },
                6
            )
        )));
    }

    let wav = std::fs::read(&out)
        .map_err(|err| MultimodalError::Runtime(format!("reading the clip: {err}")))?;
    let _ = std::fs::remove_file(&out);

    if wav.len() < 44 || &wav[0..4] != b"RIFF" {
        return Err(MultimodalError::Runtime(
            "the runtime did not produce a WAV file".to_owned(),
        ));
    }

    // The runtime reports the format on stdout; take it from the header when it
    // is absent, because the header is authoritative anyway.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let sample_rate = field(&stdout, "sample_rate=")
        .unwrap_or_else(|| u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]));
    let channels = field(&stdout, "channels=")
        .unwrap_or_else(|| u32::from(u16::from_le_bytes([wav[22], wav[23]])));

    Ok(GeneratedAudio {
        wav,
        sample_rate,
        channels,
    })
}

/// Read `key=<number>` out of the runtime's summary line.
fn field(stdout: &str, key: &str) -> Option<u32> {
    let start = stdout.find(key)? + key.len();
    let digits: String = stdout[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn temporary_wav() -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("gguf-music-{unique}.wav"))
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(prompt: &str) -> MusicRequest {
        MusicRequest::new(prompt)
    }

    #[test]
    fn an_empty_prompt_is_refused_before_spawning() {
        let error = generate(
            Path::new("no-such-audiocpp"),
            Path::new("no-such-dir"),
            &request("  "),
            AudioBackend::Cpu,
        )
        .unwrap_err();
        assert!(matches!(error, MultimodalError::Invalid(_)), "{error}");
    }

    #[test]
    fn a_zero_duration_is_refused() {
        let mut request = request("a song");
        request.duration_seconds = 0;
        let error = generate(
            Path::new("no-such-audiocpp"),
            Path::new("no-such-dir"),
            &request,
            AudioBackend::Cpu,
        )
        .unwrap_err();
        assert!(matches!(error, MultimodalError::Invalid(_)), "{error}");
    }

    #[test]
    fn a_missing_model_directory_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent");
        let error = generate(
            Path::new("no-such-audiocpp"),
            &missing,
            &request("a song"),
            AudioBackend::Cpu,
        )
        .unwrap_err();
        assert!(error.to_string().contains("model directory"), "{error}");
    }

    #[test]
    fn the_backend_name_matches_the_runtime_cli() {
        assert_eq!(AudioBackend::Cpu.as_str(), "cpu");
        assert_eq!(AudioBackend::Cuda.as_str(), "cuda");
    }

    #[test]
    fn the_defaults_are_a_short_clip() {
        assert_eq!(MusicRequest::new("x").duration_seconds, 10);
    }

    #[test]
    fn a_summary_field_is_read_from_stdout() {
        let stdout = "family=stable_audio\naudio_0 sample_rate=44100 channels=2 samples=441000";
        assert_eq!(field(stdout, "sample_rate="), Some(44_100));
        assert_eq!(field(stdout, "channels="), Some(2));
        assert_eq!(field(stdout, "missing="), None);
    }
}
