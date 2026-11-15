//! `gguf vision` - describe an image with a multimodal model (Task 8).
//!
//! Requires three files: the base weights, the `mmproj` companion that carries
//! the vision encoder, and the image. The adapter refuses before loading if any
//! is missing, because a 20 GB model load that ends in a crash is a bad answer.

use std::path::Path;

use anyhow::{Context as _, Result};
use gguf_engine_runtime_llama_cpp::probe;
use gguf_engine_runtime_llama_cpp::{VisionRequest, describe, locate_mtmd};

/// Run one vision request.
pub fn vision(
    model: &str,
    mmproj: &str,
    image: &Path,
    prompt: &str,
    max_tokens: u32,
) -> Result<bool> {
    let probed = probe::locate().context("locating llama.cpp")?;
    let Some(mtmd) = locate_mtmd(&probed.directory) else {
        anyhow::bail!(
            "llama-mtmd-cli was not found in {}; this llama.cpp build has no vision support",
            probed.directory.display()
        );
    };

    let request = VisionRequest {
        model_path: model.to_owned(),
        mmproj_path: mmproj.to_owned(),
        image_path: image.display().to_string(),
        prompt: prompt.to_owned(),
        max_tokens,
    };

    eprintln!("model    : {model}");
    eprintln!("mmproj   : {mmproj}");
    eprintln!("image    : {}", image.display());
    eprintln!("runtime  : {}", mtmd.display());
    eprintln!("loading  : this reloads the model for each request");

    let answer = describe(&mtmd, &request).context("running vision inference")?;

    println!("{answer}");
    Ok(!answer.is_empty())
}
