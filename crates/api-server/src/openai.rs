//! OpenAI-compatible request and response shapes.
//!
//! Only the fields this gateway honours are modelled. Unknown fields are
//! ignored rather than rejected, because OpenAI clients send a superset and a
//! strict schema would break them for no benefit.

use serde::{Deserialize, Serialize};

/// A message in an OpenAI chat request.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatMessage {
    /// system, user or assistant.
    pub role: String,
    /// Message text.
    pub content: String,
}

/// An OpenAI chat completion request.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionRequest {
    /// Model id.
    pub model: String,
    /// Conversation.
    pub messages: Vec<ChatMessage>,
    /// Whether to stream SSE events.
    #[serde(default)]
    pub stream: bool,
    /// Maximum tokens to generate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

/// A model entry, as `/v1/models` reports it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelEntry {
    /// Stable id.
    pub id: String,
    /// Always "model" for this API.
    pub object: String,
    /// Owner, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
}

impl ModelEntry {
    /// Build a model entry.
    pub fn new(id: impl Into<String>, owned_by: Option<String>) -> Self {
        Self {
            id: id.into(),
            object: "model".to_owned(),
            owned_by,
        }
    }
}

/// The `/v1/models` envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelList {
    /// Always "list".
    pub object: String,
    /// The models.
    pub data: Vec<ModelEntry>,
}

impl ModelList {
    /// Wrap a list of models.
    pub fn new(data: Vec<ModelEntry>) -> Self {
        Self {
            object: "list".to_owned(),
            data,
        }
    }
}

/// Token accounting, as OpenAI reports it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    /// Prompt tokens. Reported as 0 when the runtime does not count them.
    pub prompt_tokens: u32,
    /// Completion tokens actually generated.
    pub completion_tokens: u32,
    /// Sum of the two.
    pub total_tokens: u32,
}

/// One choice in a non-streaming completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    /// Index, always 0.
    pub index: u32,
    /// The message.
    pub message: ChatMessage,
    /// Why generation stopped.
    pub finish_reason: String,
}

/// A non-streaming completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    /// Response id.
    pub id: String,
    /// Always "chat.completion".
    pub object: String,
    /// Creation time, unix seconds.
    pub created: u64,
    /// Model used.
    pub model: String,
    /// Choices.
    pub choices: Vec<Choice>,
    /// Token accounting.
    pub usage: Usage,
}

/// The `delta` object of a streaming chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    /// Role, sent on the first chunk only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Text of this chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// One choice in a streaming chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkChoice {
    /// Index, always 0.
    pub index: u32,
    /// The delta.
    pub delta: Delta,
    /// Set on the final chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// A streaming chunk (`data: {...}` in the SSE stream).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunk {
    /// Response id, identical across the stream.
    pub id: String,
    /// Always "chat.completion.chunk".
    pub object: String,
    /// Creation time, unix seconds.
    pub created: u64,
    /// Model used.
    pub model: String,
    /// Choices, exactly one.
    pub choices: Vec<ChunkChoice>,
}

/// An OpenAI-shaped error body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    /// The error.
    pub error: ErrorDetail,
}

/// The error detail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    /// Human-readable message.
    pub message: String,
    /// Machine-readable type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Parameter at fault, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    /// Error code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}
