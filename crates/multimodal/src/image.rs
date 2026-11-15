//! Image generation through stable-diffusion.cpp's HTTP server.
//!
//! The server speaks an OpenAI-shaped `POST /v1/images/generations` and answers
//! with base64-encoded image bytes. That is the same protocol shape the gateway
//! already uses for chat, so this adapter is a thin translation and nothing
//! else - the model loading, sampling and VAE work belong to the runtime.

use serde::{Deserialize, Serialize};

use crate::error::MultimodalError;

/// What to draw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageRequest {
    /// The prompt.
    pub prompt: String,
    /// What to avoid. Empty is passed through as empty rather than omitted.
    pub negative_prompt: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Sampling steps.
    pub steps: u32,
    /// Seed; `None` lets the runtime choose.
    pub seed: Option<i64>,
}

impl ImageRequest {
    /// A request with the defaults this project uses.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            negative_prompt: String::new(),
            width: 512,
            height: 512,
            steps: 20,
            seed: None,
        }
    }
}

/// One generated image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    /// Encoded bytes, exactly as the runtime produced them.
    pub bytes: Vec<u8>,
    /// Format the runtime used, e.g. `png`.
    pub format: String,
}

/// Generate one image.
///
/// Errors carry the runtime's own message: "HTTP 500" tells a user nothing, and
/// the server usually explains itself (unsupported size, out of memory, ...).
pub async fn generate(
    base_url: &str,
    request: &ImageRequest,
) -> Result<GeneratedImage, MultimodalError> {
    if request.prompt.trim().is_empty() {
        return Err(MultimodalError::Invalid(
            "the prompt must not be empty".to_owned(),
        ));
    }
    if request.width == 0 || request.height == 0 {
        return Err(MultimodalError::Invalid(
            "width and height must not be zero".to_owned(),
        ));
    }

    let body = serde_json::json!({
        "prompt": request.prompt,
        "negative_prompt": request.negative_prompt,
        "size": format!("{}x{}", request.width, request.height),
        "n": 1,
        "output_format": "png",
        "steps": request.steps,
        "seed": request.seed,
    });

    // A diffusion model producing 512x512 in 20 steps takes tens of seconds on a
    // GPU and minutes on a CPU; a short timeout would report a slow success as a
    // failure.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|err| MultimodalError::Runtime(format!("building the HTTP client: {err}")))?;

    let response = client
        .post(format!("{base_url}/v1/images/generations"))
        .json(&body)
        .send()
        .await
        .map_err(|err| MultimodalError::Runtime(format!("requesting an image: {err}")))?;

    let status = response.status();
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|err| MultimodalError::Runtime(format!("reading the image reply: {err}")))?;

    if !status.is_success() {
        return Err(MultimodalError::Runtime(format!(
            "the runtime refused the request ({status}): {}",
            runtime_error_message(&payload)
        )));
    }

    let encoded = payload
        .get("data")
        .and_then(|data| data.as_array())
        .and_then(|items| items.first())
        .and_then(|item| item.get("b64_json"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            MultimodalError::Runtime(format!(
                "the runtime returned no image: {}",
                runtime_error_message(&payload)
            ))
        })?;

    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|err| MultimodalError::Runtime(format!("decoding the image: {err}")))?;

    if bytes.is_empty() {
        return Err(MultimodalError::Runtime(
            "the runtime returned an empty image".to_owned(),
        ));
    }

    Ok(GeneratedImage {
        bytes,
        format: "png".to_owned(),
    })
}

/// Pull a human-readable message out of an error payload.
///
/// sd.cpp uses `error` as a string on some paths and as an object on others, so
/// both shapes are handled rather than one being assumed.
fn runtime_error_message(payload: &serde_json::Value) -> String {
    match payload.get("error") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Object(map)) => map
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("no message")
            .to_owned(),
        _ => payload.to_string().chars().take(300).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_prompt_is_refused_before_any_request() {
        let request = ImageRequest::new("  ");
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(generate("http://127.0.0.1:1", &request))
            .unwrap_err();
        assert!(matches!(error, MultimodalError::Invalid(_)), "{error}");
    }

    #[test]
    fn a_zero_size_is_refused() {
        let mut request = ImageRequest::new("a cat");
        request.width = 0;
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(generate("http://127.0.0.1:1", &request))
            .unwrap_err();
        assert!(matches!(error, MultimodalError::Invalid(_)), "{error}");
    }

    #[test]
    fn the_defaults_are_a_small_fast_image() {
        let request = ImageRequest::new("a cat");
        assert_eq!(request.width, 512);
        assert_eq!(request.height, 512);
        assert_eq!(request.steps, 20);
        assert!(request.seed.is_none(), "the runtime should choose");
    }

    #[test]
    fn an_error_payload_is_read_in_both_shapes() {
        let text = serde_json::json!({ "error": "out of memory" });
        assert_eq!(runtime_error_message(&text), "out of memory");

        let object = serde_json::json!({ "error": { "message": "bad size" } });
        assert_eq!(runtime_error_message(&object), "bad size");

        let bare = serde_json::json!({ "detail": "unknown" });
        assert!(runtime_error_message(&bare).contains("detail"));
    }
}
