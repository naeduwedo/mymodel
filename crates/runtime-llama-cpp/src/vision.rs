//! Vision adapter over llama.cpp `mtmd` (spec 8.3, Task 8).
//!
//! llama.cpp keeps the vision encoder in a separate `mmproj-*.gguf` companion and
//! exposes image input through the `llama-mtmd-cli` tool. That tool is one-shot:
//! it loads the model, encodes the image, answers, and exits. So this adapter
//! spawns it per request and captures its output, which is honest about the
//! cost (a model reload per call) instead of pretending to hold a session.

use std::path::{Path, PathBuf};
use std::process::Command;

use gguf_engine_runtime_manager::{Modality, RuntimeError};

/// Name of the mtmd executable on this platform.
pub fn mtmd_binary_name() -> &'static str {
    if cfg!(windows) {
        "llama-mtmd-cli.exe"
    } else {
        "llama-mtmd-cli"
    }
}

/// Locate the mtmd executable next to a known llama.cpp installation.
pub fn locate_mtmd(llama_dir: &Path) -> Option<PathBuf> {
    let candidate = llama_dir.join(mtmd_binary_name());
    candidate.is_file().then_some(candidate)
}

/// A vision request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisionRequest {
    /// Path to the base model weights.
    pub model_path: String,
    /// Path to the `mmproj` companion that carries the vision encoder.
    pub mmproj_path: String,
    /// Path to the image to describe.
    pub image_path: String,
    /// The question to ask about the image.
    pub prompt: String,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
}

/// Validate a request before spawning anything.
///
/// A vision model without its projector cannot answer, and saying so here is far
/// better than a 30-second model load that ends in a crash.
pub fn validate(request: &VisionRequest) -> Result<(), RuntimeError> {
    for (label, path) in [
        ("model", &request.model_path),
        ("mmproj", &request.mmproj_path),
        ("image", &request.image_path),
    ] {
        if !Path::new(path).is_file() {
            return Err(RuntimeError::NotFound(format!("{label} file: {path}")));
        }
    }

    if request.prompt.trim().is_empty() {
        return Err(RuntimeError::Start("prompt must not be empty".to_owned()));
    }

    Ok(())
}

/// Strip llama.cpp's timing/log noise from mtmd stdout.
///
/// The tool prints progress and warnings to stderr and the answer to stdout, but
/// some builds also emit timing lines on stdout. Keeping the cleaning in one
/// place means the adapter returns text a user could actually paste.
pub fn clean_output(stdout: &str) -> String {
    stdout
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && !trimmed.starts_with("llama_")
                && !trimmed.contains("tokens per second")
                && !trimmed.contains("load time")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// Run one vision request, returning the model's answer.
pub fn describe(mtmd_path: &Path, request: &VisionRequest) -> Result<String, RuntimeError> {
    validate(request)?;

    let output = Command::new(mtmd_path)
        .arg("-m")
        .arg(&request.model_path)
        .arg("--mmproj")
        .arg(&request.mmproj_path)
        .arg("--image")
        .arg(&request.image_path)
        .arg("-p")
        .arg(&request.prompt)
        .arg("-n")
        .arg(request.max_tokens.to_string())
        .arg("--temp")
        .arg("0")
        .output()
        .map_err(|err| RuntimeError::Start(format!("spawning mtmd: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(8).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(RuntimeError::WorkerExited {
            code: output.status.code(),
            stderr: tail.join("\n"),
        });
    }

    Ok(clean_output(&String::from_utf8_lossy(&output.stdout)))
}

/// The modality this adapter provides.
pub fn modality() -> Modality {
    Modality::Vision
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(model: &str, mmproj: &str, image: &str) -> VisionRequest {
        VisionRequest {
            model_path: model.to_owned(),
            mmproj_path: mmproj.to_owned(),
            image_path: image.to_owned(),
            prompt: "what is this?".to_owned(),
            max_tokens: 16,
        }
    }

    #[test]
    fn a_missing_projector_is_reported_by_name() {
        // The model and image exist; only the projector is absent. That is the
        // realistic failure: a user downloaded the weights but not the encoder.
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("m.gguf");
        let image = dir.path().join("i.png");
        std::fs::write(&model, b"x").unwrap();
        std::fs::write(&image, b"x").unwrap();
        let missing_mmproj = dir.path().join("absent-mmproj.gguf");

        let err = validate(&request(
            &model.display().to_string(),
            &missing_mmproj.display().to_string(),
            &image.display().to_string(),
        ))
        .unwrap_err();
        assert!(err.to_string().contains("mmproj"), "{err}");
    }

    #[test]
    fn an_empty_prompt_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("m.gguf");
        let mmproj = dir.path().join("p.gguf");
        let image = dir.path().join("i.png");
        for path in [&model, &mmproj, &image] {
            std::fs::write(path, b"x").unwrap();
        }

        let mut req = request(
            &model.display().to_string(),
            &mmproj.display().to_string(),
            &image.display().to_string(),
        );
        req.prompt = "  ".to_owned();
        assert!(validate(&req).is_err());
    }

    #[test]
    fn a_complete_request_validates() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("m.gguf");
        let mmproj = dir.path().join("p.gguf");
        let image = dir.path().join("i.png");
        for path in [&model, &mmproj, &image] {
            std::fs::write(path, b"x").unwrap();
        }
        validate(&request(
            &model.display().to_string(),
            &mmproj.display().to_string(),
            &image.display().to_string(),
        ))
        .expect("all three files exist");
    }

    #[test]
    fn timing_noise_is_stripped_from_the_answer() {
        let raw =
            "llama_model_loader: loaded\nThe shape is yellow.\nllama_perf: tokens per second: 12";
        assert_eq!(clean_output(raw), "The shape is yellow.");
    }

    #[test]
    fn mtmd_binary_name_matches_the_platform() {
        if cfg!(windows) {
            assert_eq!(mtmd_binary_name(), "llama-mtmd-cli.exe");
        } else {
            assert_eq!(mtmd_binary_name(), "llama-mtmd-cli");
        }
    }
}
