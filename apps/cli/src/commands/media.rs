//! `gguf image`, `gguf video`, `gguf transcribe`, `gguf speak`.
//!
//! Each command resolves its runtime by probing (never by assumption), then
//! either runs or says precisely what is missing.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gguf_engine_multimodal::{
    ImageRequest, MultimodalError, generate_image, probe_all, synthesise, transcribe,
};

use crate::commands::Context;
use crate::output;

/// Find the installed runtime that provides `modality`, or explain why not.
fn runtime_for(modality: &str) -> Result<&'static gguf_engine_multimodal::RuntimeProbe> {
    for probe in probe_all() {
        if probe.installed && probe.manifest.modalities.iter().any(|m| m == modality) {
            return Ok(probe);
        }
    }

    // Report every runtime that claims the modality and why each failed, so
    // the message names the actual gap.
    let mut reasons = Vec::new();
    for probe in probe_all() {
        if probe.manifest.modalities.iter().any(|m| m == modality) {
            reasons.push(format!(
                "{}: {}",
                probe.manifest.display_name,
                probe.reason.as_deref().unwrap_or("not installed")
            ));
        }
    }

    if reasons.is_empty() {
        anyhow::bail!("no runtime in this build provides {modality}");
    }
    anyhow::bail!(
        "no runtime for {modality} is installed:\n  {}",
        reasons.join("\n  ")
    )
}

/// Like `require_path`, but for a configured directory.
fn require_dir(label: &str, value: Option<&String>) -> Result<PathBuf> {
    match value {
        Some(path) if !path.trim().is_empty() => {
            let path = PathBuf::from(path);
            if !path.is_dir() {
                anyhow::bail!(
                    "{label} must be a model directory, not a file: {}",
                    path.display()
                );
            }
            Ok(path)
        }
        _ => anyhow::bail!("set multimodal.{label} in the configuration (gguf config --show)"),
    }
}

fn require_path(label: &str, value: Option<&String>) -> Result<PathBuf> {
    match value {
        Some(path) if !path.trim().is_empty() => {
            let path = PathBuf::from(path);
            if !path.is_file() {
                anyhow::bail!("{label} not found: {}", path.display());
            }
            Ok(path)
        }
        _ => anyhow::bail!("set multimodal.{label} in the configuration (gguf config --show)"),
    }
}

/// `gguf image` — generate an image.
pub async fn image(ctx: &Context, prompt: &str, options: ImageOptions, json: bool) -> Result<bool> {
    let probe = runtime_for("image_generation")?;
    let base = ctx
        .config
        .multimodal
        .diffusion_url
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .or_else(|| gguf_engine_multimodal::http_base(probe))
        .context("the image runtime does not expose an HTTP endpoint")?;

    let _model = require_path("image_model", ctx.config.multimodal.image_model.as_ref())?;

    let mut request = ImageRequest::new(prompt);
    request.width = options.width;
    request.height = options.height;
    request.steps = options.steps;
    request.seed = options.seed;
    request.negative_prompt = options.negative_prompt.unwrap_or_default();

    if !json {
        eprintln!("runtime  : {} ({base})", probe.manifest.display_name);
        eprintln!(
            "request  : {}x{}, {} steps",
            request.width, request.height, request.steps
        );
    }

    let generated = generate_image(&base, &request).await.map_err(describe)?;

    let out = PathBuf::from(&options.out);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&out, &generated.bytes).with_context(|| format!("writing {}", out.display()))?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "path": out.display().to_string(),
                "bytes": generated.bytes.len(),
                "format": generated.format,
            })
        );
    } else {
        println!(
            "wrote {} ({})",
            out.display(),
            output::human_bytes(generated.bytes.len() as u64)
        );
    }

    Ok(true)
}

/// `gguf video` — generate a clip.
pub async fn video(ctx: &Context, prompt: &str, options: VideoOptions, json: bool) -> Result<bool> {
    let probe = runtime_for("video")?;
    let base = ctx
        .config
        .multimodal
        .diffusion_url
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .or_else(|| gguf_engine_multimodal::http_base(probe))
        .context("the video runtime does not expose an HTTP endpoint")?;

    let mut request = gguf_engine_multimodal::VideoRequest::new(prompt);
    request.width = options.width;
    request.height = options.height;
    request.frames = options.frames;
    request.fps = options.fps;
    request.steps = options.steps;
    request.seed = options.seed;

    if !json {
        eprintln!("runtime  : {} ({base})", probe.manifest.display_name);
        eprintln!(
            "request  : {}x{}, {} frames at {} fps",
            request.width, request.height, request.frames, request.fps
        );
        eprintln!("note     : a clip takes minutes; polling the runtime");
    }

    // Progress goes to stderr so `--json` keeps producing one clean document on
    // stdout, and so a piped caller is not given a stream of percentages.
    let mut last_line = String::new();
    let mut last_key = String::new();
    let generated =
        gguf_engine_multimodal::generate_video_with_progress(&base, &request, |progress| {
            // Only rewrite when the stage or the step count changes. The elapsed time
            // changes on every poll, so including it in the comparison would emit a
            // line per second - harmless in a terminal, noise in a redirected log.
            let key = format!(
                "{}/{:?}/{}",
                progress.stage, progress.completed, progress.status
            );
            if key != last_key
                && let Some(line) = describe_progress(&progress)
            {
                eprint!("\r{line}");
                last_line = line;
                last_key = key;
            }
        })
        .await
        .map_err(describe)?;

    if !json && !last_line.is_empty() {
        eprintln!();
    }

    // The runtime chooses the container (webm for sd.cpp). Writing those bytes to
    // a ".mp4" name would produce a file players refuse, so the extension follows
    // the MIME type the runtime reported.
    let requested = PathBuf::from(&options.out);
    let out = match container_extension(&generated.mime_type) {
        Some(extension) if requested.extension().and_then(|e| e.to_str()) != Some(extension) => {
            let corrected = requested.with_extension(extension);
            eprintln!(
                "note     : the runtime produced {}, so the file is {}",
                generated.mime_type,
                corrected.display()
            );
            corrected
        }
        _ => requested,
    };
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&out, &generated.bytes).with_context(|| format!("writing {}", out.display()))?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "path": out.display().to_string(),
                "bytes": generated.bytes.len(),
                "mime_type": generated.mime_type,
                "frames": generated.frames,
            })
        );
    } else {
        println!(
            "wrote {} ({}, {} frames)",
            out.display(),
            output::human_bytes(generated.bytes.len() as u64),
            generated.frames
        );
    }

    Ok(true)
}

/// Video generation options.
#[derive(Debug, Clone)]
pub struct VideoOptions {
    /// Output path.
    pub out: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Frame count.
    pub frames: u32,
    /// Frames per second.
    pub fps: u32,
    /// Sampling steps.
    pub steps: u32,
    /// Seed.
    pub seed: Option<i64>,
}

/// Image generation options.
#[derive(Debug, Clone)]
pub struct ImageOptions {
    /// Output path.
    pub out: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Sampling steps.
    pub steps: u32,
    /// Seed.
    pub seed: Option<i64>,
    /// Negative prompt.
    pub negative_prompt: Option<String>,
}

/// `gguf music` — generate music or sound effects.
///
/// Runs on the ggml audio runtime (`audio.cpp`): GGUF weights, no Python and no
/// ComfyUI. The call blocks for the render, which is seconds to a couple of
/// minutes depending on length and backend.
pub fn music(ctx: &Context, prompt: &str, options: MusicOptions, json: bool) -> Result<bool> {
    let probe = runtime_for("music")?;
    let binary = probe
        .path
        .as_deref()
        .context("the music runtime has no path")?;
    // The music runtime resolves components inside a package directory, so this
    // one is a directory, not a file.
    let model = require_dir("music_model", ctx.config.multimodal.music_model.as_ref())?;

    let backend = match ctx.config.multimodal.audio_backend.as_deref() {
        Some("cpu") => gguf_engine_multimodal::AudioBackend::Cpu,
        Some("cuda") => gguf_engine_multimodal::AudioBackend::Cuda,
        _ => gguf_engine_multimodal::AudioBackend::default(),
    };

    let mut request = gguf_engine_multimodal::MusicRequest::new(prompt);
    request.duration_seconds = options.duration_seconds;

    if !json {
        eprintln!(
            "runtime  : {} ({}, {:?})",
            probe.manifest.display_name,
            binary.display(),
            backend
        );
        eprintln!("model    : {}", model.display());
        eprintln!("request  : {} s", request.duration_seconds);
    }

    let generated = gguf_engine_multimodal::generate_music(binary, &model, &request, backend)
        .map_err(describe)?;

    if let Some(parent) = options.out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&options.out, &generated.wav)
        .with_context(|| format!("writing {}", options.out.display()))?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "path": options.out.display().to_string(),
                "bytes": generated.wav.len(),
                "sample_rate": generated.sample_rate,
                "channels": generated.channels,
            })
        );
    } else {
        println!(
            "wrote {} ({}, {} Hz, {} channel(s))",
            options.out.display(),
            output::human_bytes(generated.wav.len() as u64),
            generated.sample_rate,
            generated.channels
        );
    }

    Ok(true)
}

/// Music generation options.
#[derive(Debug, Clone)]
pub struct MusicOptions {
    /// Output path.
    pub out: PathBuf,
    /// Length in seconds.
    pub duration_seconds: u32,
}

/// `gguf transcribe` — speech to text.
pub fn transcribe_audio(
    ctx: &Context,
    audio: &Path,
    language: Option<&str>,
    json: bool,
) -> Result<bool> {
    let probe = runtime_for("asr")?;
    let binary = probe
        .path
        .as_deref()
        .context("the ASR runtime has no path")?;
    let model = require_path("asr_model", ctx.config.multimodal.asr_model.as_ref())?;

    if !json {
        eprintln!(
            "runtime  : {} ({})",
            probe.manifest.display_name,
            binary.display()
        );
        eprintln!("audio    : {}", audio.display());
    }

    let result = transcribe(binary, &model, audio, language).map_err(describe)?;

    if json {
        println!("{}", serde_json::json!({ "ok": true, "text": result.text }));
    } else {
        println!("{}", result.text);
    }

    Ok(!result.text.is_empty())
}

/// `gguf speak` — text to speech.
pub fn speak(ctx: &Context, text: &str, out: &Path, json: bool) -> Result<bool> {
    let probe = runtime_for("tts")?;
    let binary = probe
        .path
        .as_deref()
        .context("the TTS runtime has no path")?;
    let voice = require_path("tts_voice", ctx.config.multimodal.tts_voice.as_ref())?;

    if !json {
        eprintln!(
            "runtime  : {} ({})",
            probe.manifest.display_name,
            binary.display()
        );
        eprintln!("voice    : {}", voice.display());
    }

    let speech = synthesise(binary, &voice, text).map_err(describe)?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(out, &speech.wav).with_context(|| format!("writing {}", out.display()))?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "path": out.display().to_string(),
                "bytes": speech.wav.len(),
            })
        );
    } else {
        println!(
            "wrote {} ({})",
            out.display(),
            output::human_bytes(speech.wav.len() as u64)
        );
    }

    Ok(true)
}

/// One line describing a render's progress, or `None` when there is nothing new.
///
/// A percentage is only shown when the runtime reports both a completed and a
/// total count: deriving one from elapsed time would be a guess that gets more
/// wrong the slower the machine is.
pub fn describe_progress(progress: &gguf_engine_multimodal::VideoProgress) -> Option<String> {
    let head = if progress.queue_position > 0 {
        format!("queued (position {})", progress.queue_position)
    } else if progress.stage.is_empty() {
        progress.status.clone()
    } else {
        progress.stage.clone()
    };

    let counts = match (progress.completed, progress.total) {
        (Some(done), Some(total)) if total > 0 => {
            let unit = progress.unit.as_deref().unwrap_or("items");
            format!(" {done}/{total} {unit}")
        }
        (Some(done), _) => format!(" {done}"),
        _ => String::new(),
    };

    let percent = progress
        .fraction()
        .map(|fraction| format!(" {:.0}%", fraction * 100.0))
        .unwrap_or_default();

    let seconds = progress.elapsed_ms / 1000;
    Some(format!("{head}{counts}{percent} ({seconds}s)"))
}

/// The file extension for a container MIME type.
///
/// Returns `None` for types this build does not recognise, in which case the
/// caller keeps whatever name it was given rather than inventing one.
pub fn container_extension(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "video/webm" => Some("webm"),
        "video/mp4" => Some("mp4"),
        "image/png" => Some("png"),
        _ => None,
    }
}

fn describe(error: MultimodalError) -> anyhow::Error {
    anyhow::anyhow!(error.to_string())
}
