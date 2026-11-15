//! Speech recognition and synthesis.
//!
//! Both runtimes here are command-line tools, not servers: whisper.cpp's CLI and
//! Piper. Each request spawns the tool, which is honest about the cost - whisper
//! loads a model per invocation - and avoids keeping a process resident for a
//! feature a user may call once.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::error::MultimodalError;

/// A transcription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcription {
    /// The recognised text.
    pub text: String,
}

/// Transcribe an audio file with whisper.cpp.
///
/// `language` is passed through when set; whisper decides otherwise. The tool
/// writes progress to stderr and the transcript to stdout, so only stdout is
/// read - mixing the two would put "whisper_init_from_file..." in the answer.
pub fn transcribe(
    binary: &Path,
    model: &Path,
    audio: &Path,
    language: Option<&str>,
) -> Result<Transcription, MultimodalError> {
    for (label, path) in [("model", model), ("audio", audio)] {
        if !path.is_file() {
            return Err(MultimodalError::Invalid(format!(
                "{label} file not found: {}",
                path.display()
            )));
        }
    }

    let mut command = std::process::Command::new(binary);
    command
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(audio)
        // Plain text: without this every line is prefixed with a timestamp, and
        // the caller would have to strip them to get a sentence.
        .arg("-nt")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());

    if let Some(language) = language {
        command.arg("-l").arg(language);
    }

    let output = command
        .output()
        .map_err(|err| MultimodalError::Runtime(format!("running whisper: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MultimodalError::Runtime(format!(
            "whisper failed: {}",
            tail(&stderr, 6)
        )));
    }

    let text = clean_transcript(&String::from_utf8_lossy(&output.stdout));
    if text.is_empty() {
        // Silence is a legitimate result, but an empty answer is more often a
        // wrong model or a wrong format, so it is reported rather than returned
        // as a successful empty string.
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MultimodalError::Runtime(format!(
            "whisper produced no text; its output was: {}",
            tail(&stderr, 6)
        )));
    }

    Ok(Transcription { text })
}

/// A synthesised voice clip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    /// WAV bytes.
    pub wav: Vec<u8>,
}

/// Synthesise speech with Piper.
///
/// Piper reads the text from stdin and writes a WAV to `--output_file`; it has no
/// stdout mode, so a temporary file is used and then read back.
pub fn synthesise(binary: &Path, voice: &Path, text: &str) -> Result<Speech, MultimodalError> {
    if text.trim().is_empty() {
        return Err(MultimodalError::Invalid(
            "the text must not be empty".to_owned(),
        ));
    }
    if !voice.is_file() {
        return Err(MultimodalError::Invalid(format!(
            "voice model not found: {}",
            voice.display()
        )));
    }

    let out = temporary_wav_path();
    let mut child = std::process::Command::new(binary)
        .arg("--model")
        .arg(voice)
        .arg("--output_file")
        .arg(&out)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|err| MultimodalError::Runtime(format!("running piper: {err}")))?;

    {
        use std::io::Write as _;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| MultimodalError::Runtime("piper has no stdin".to_owned()))?;
        stdin
            .write_all(text.as_bytes())
            .map_err(|err| MultimodalError::Runtime(format!("writing to piper: {err}")))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|err| MultimodalError::Runtime(format!("waiting for piper: {err}")))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&out);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(MultimodalError::Runtime(format!(
            "piper failed: {}",
            tail(&stderr, 6)
        )));
    }

    let wav = std::fs::read(&out)
        .map_err(|err| MultimodalError::Runtime(format!("reading the synthesised audio: {err}")))?;
    let _ = std::fs::remove_file(&out);

    if wav.is_empty() {
        return Err(MultimodalError::Runtime(
            "piper produced no audio".to_owned(),
        ));
    }

    Ok(Speech { wav })
}

/// Reduce whisper's stdout to the transcript.
///
/// Even with `-nt` the tool prints blank lines around segments, and a caller who
/// has to trim those is a caller who will forget to.
pub fn clean_transcript(stdout: &str) -> String {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn temporary_wav_path() -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("gguf-tts-{unique}.wav"))
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

    #[test]
    fn blank_lines_are_not_part_of_the_transcript() {
        let stdout = "\n Hello there.\n\n General Kenobi. \n\n";
        assert_eq!(clean_transcript(stdout), "Hello there. General Kenobi.");
    }

    #[test]
    fn an_empty_transcript_stays_empty() {
        assert_eq!(clean_transcript("\n\n  \n"), "");
    }

    #[test]
    fn a_missing_audio_file_is_refused_before_spawning() {
        let error = transcribe(
            Path::new("no-such-whisper"),
            Path::new("no-such-model.bin"),
            Path::new("no-such-audio.wav"),
            None,
        )
        .unwrap_err();
        assert!(matches!(error, MultimodalError::Invalid(_)), "{error}");
    }

    #[test]
    fn empty_text_is_refused_before_spawning() {
        let error = synthesise(
            Path::new("no-such-piper"),
            Path::new("no-such-voice.onnx"),
            "  ",
        )
        .unwrap_err();
        assert!(matches!(error, MultimodalError::Invalid(_)), "{error}");
    }

    #[test]
    fn tail_keeps_the_last_lines_only() {
        assert_eq!(tail("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(tail("only", 3), "only");
    }
}
