//! HTTP client for llama-server's OpenAI-compatible endpoints.
//!
//! Only what Task 6 needs is here: /health and /completion (streaming). The
//! server speaks Server-Sent Events: one 'data: {json}' line per chunk, then a
//! blank line. The final chunk has stop=true and empty content.

use futures::StreamExt as _;
use gguf_engine_runtime_manager::{ChatMessage, RuntimeError, Token};
use serde::Deserialize;

/// One line of a /completion SSE stream, decoded.
#[derive(Debug, Deserialize)]
struct CompletionChunk {
    /// Text of this chunk.
    #[serde(default)]
    content: String,
    /// Whether generation finished.
    #[serde(default)]
    stop: bool,
}

/// The body sent to /completion.
#[derive(Debug, serde::Serialize)]
struct CompletionBody {
    prompt: String,
    n_predict: u32,
    temperature: f32,
    stream: bool,
}

/// True when the worker reports healthy.
pub async fn healthy(base: &str) -> bool {
    let url = format!("{base}/health");
    match reqwest::Client::new().get(&url).send().await {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

/// Convert a chat history into a plain prompt.
fn prompt_from_messages(messages: &[ChatMessage]) -> String {
    let mut prompt = String::new();
    for message in messages {
        match message.role.as_str() {
            "system" => {
                prompt.push_str("System: ");
                prompt.push_str(&message.content);
                prompt.push('\n');
                prompt.push('\n');
            }
            "user" => {
                prompt.push_str("User: ");
                prompt.push_str(&message.content);
                prompt.push('\n');
            }
            "assistant" => {
                prompt.push_str("Assistant: ");
                prompt.push_str(&message.content);
                prompt.push('\n');
            }
            _ => {
                prompt.push_str(&message.content);
                prompt.push('\n');
            }
        }
    }
    prompt.push_str("Assistant:");
    prompt
}

/// POST /completion and return the full text (non-streaming).
pub async fn completion(
    base: &str,
    messages: &[ChatMessage],
    max_tokens: u32,
    temperature: f32,
) -> Result<String, RuntimeError> {
    let body = CompletionBody {
        prompt: prompt_from_messages(messages),
        n_predict: max_tokens,
        temperature,
        stream: false,
    };

    let url = format!("{base}/completion");
    let response = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|err| RuntimeError::Http(err.to_string()))?;

    if !response.status().is_success() {
        return Err(RuntimeError::Http(format!(
            "{}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        )));
    }

    let chunk: CompletionChunk = response
        .json()
        .await
        .map_err(|err| RuntimeError::Http(err.to_string()))?;
    Ok(chunk.content)
}

/// Stream a /completion as a stream of tokens.
pub fn stream_completion(
    base: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
) -> gguf_engine_runtime_manager::TokenStream {
    let body = CompletionBody {
        prompt: prompt_from_messages(&messages),
        n_predict: max_tokens,
        temperature,
        stream: true,
    };

    let stream = async_stream::try_stream! {
        let url = format!("{base}/completion");
        let response = reqwest::Client::new()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|err| RuntimeError::Http(err.to_string()))?;

        // On a non-success status, report the code and point at the worker
        // log; reading the body here would consume the response before the
        // stream below, which the borrow checker rightly rejects.
        let status = response.status();
        if !status.is_success() {
            Err(RuntimeError::Http(format!(
                "HTTP {status}; see the worker stderr log for details"
            )))?;
        }

        let mut lines = response.bytes_stream();
        let mut pending = String::new();
        while let Some(bytes) = lines.next().await {
            let bytes = bytes.map_err(|err| RuntimeError::Http(err.to_string()))?;
            pending.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(pos) = pending.find("\n\n") {
                let frame = pending[..pos].to_owned();
                pending.drain(..pos + 2);
                for line in frame.lines() {
                    let Some(data) = line.strip_prefix("data: ") else { continue };
                    let chunk: CompletionChunk = serde_json::from_str(data)
                        .map_err(|err| RuntimeError::Http(err.to_string()))?;
                    if chunk.stop { return; }
                    if !chunk.content.is_empty() {
                        yield Token { content: chunk.content };
                    }
                }
            }
        }
    };

    Box::pin(stream)
}

/// Ask the worker for embeddings of `inputs`.
///
/// Uses llama.cpp's native `/embedding` endpoint. It takes one `content` per
/// request, so several inputs are sent one at a time: batching them is not
/// supported by this endpoint, and pretending otherwise would silently return
/// fewer vectors than were asked for.
pub async fn embeddings(base_url: &str, inputs: &[String]) -> Result<Vec<Vec<f32>>, RuntimeError> {
    let client = reqwest::Client::new();
    let mut vectors = Vec::with_capacity(inputs.len());

    for input in inputs {
        let response = client
            .post(format!("{base_url}/embedding"))
            .json(&serde_json::json!({ "content": input }))
            .send()
            .await
            .map_err(|err| RuntimeError::Start(format!("requesting embeddings: {err}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            // A chat model started without --embeddings answers like this, so
            // the message must say what happened rather than "HTTP 400".
            return Err(RuntimeError::Start(format!(
                "the worker refused an embedding request ({status}): {}",
                text.chars().take(300).collect::<String>()
            )));
        }

        let parsed: serde_json::Value = response
            .json()
            .await
            .map_err(|err| RuntimeError::Start(format!("reading embeddings: {err}")))?;

        // llama.cpp answers with [{index, embedding: [[...]]}]; the inner nesting
        // is one batch, so the first element is this input's vector.
        let vector = parsed
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("embedding"))
            .and_then(|embedding| embedding.as_array())
            .and_then(|batch| batch.first())
            .and_then(|first| first.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_f64)
                    .map(|value| value as f32)
                    .collect::<Vec<f32>>()
            })
            .unwrap_or_default();

        vectors.push(vector);
    }

    Ok(vectors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_owned(),
            content: content.to_owned(),
        }
    }

    #[test]
    fn prompt_folds_system_first_and_ends_with_assistant_marker() {
        let messages = vec![msg("system", "You are helpful."), msg("user", "hi")];
        let prompt = prompt_from_messages(&messages);
        assert!(prompt.starts_with("System: You are helpful."));
        assert!(prompt.contains("User: hi"));
        assert!(prompt.ends_with("Assistant:"));
    }

    #[test]
    fn unknown_roles_are_included_verbatim() {
        let prompt = prompt_from_messages(&[msg("tool", "result")]);
        assert!(prompt.contains("result"));
    }
}
