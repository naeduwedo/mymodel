//! The boundary between HTTP and the inference engine.
//!
//! The gateway does **not** load models itself. It translates OpenAI JSON into a
//! request the core already understands and relays the token stream back. That
//! keeps one inference path in the product instead of two that can drift.

use std::pin::Pin;

use async_trait::async_trait;
use tokio_stream::Stream;

use crate::error::ApiError;
use crate::openai::{ChatMessage, ModelEntry};

/// A stream of text deltas.
pub type ChunkStream = Pin<Box<dyn Stream<Item = Result<String, ApiError>> + Send>>;

/// What the gateway needs from the engine.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Models the gateway may serve.
    async fn models(&self) -> Result<Vec<ModelEntry>, ApiError>;

    /// Start a chat completion, returning deltas as they are generated.
    ///
    /// Streaming is the only mode. Non-streaming responses are produced by
    /// draining this same stream, so the two modes cannot disagree about what
    /// the model said.
    async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<ChunkStream, ApiError>;
}

/// A backend over the core's local gRPC transport.
#[derive(Debug, Clone)]
pub struct IpcBackend {
    data_root: std::path::PathBuf,
}

impl IpcBackend {
    /// Talk to the core serving `data_root`.
    pub fn new(data_root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    async fn client(&self) -> Result<gguf_engine_ipc::client::CoreClient, ApiError> {
        gguf_engine_ipc::client::connect(&self.data_root)
            .await
            .map_err(|err| ApiError::CoreUnavailable(err.to_string()))
    }
}

#[async_trait]
impl Backend for IpcBackend {
    async fn models(&self) -> Result<Vec<ModelEntry>, ApiError> {
        let client = self.client().await?;
        let listed = client
            .list_models()
            .await
            .map_err(|err| ApiError::CoreUnavailable(err.to_string()))?;

        // Only what the registry holds: an entry here is a model a client may
        // actually ask for, not a catalogue entry from the hub.
        Ok(listed
            .models
            .into_iter()
            .map(|model| {
                let owner = (!model.architecture.is_empty()).then_some(model.architecture);
                ModelEntry::new(model.model_id, owner)
            })
            .collect())
    }

    async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<ChunkStream, ApiError> {
        let client = self.client().await?;

        let request = gguf_engine_protocol::pb::ChatRequest {
            model_id: model.to_owned(),
            messages: messages
                .into_iter()
                .map(|message| gguf_engine_protocol::pb::ChatMessage {
                    role: message.role,
                    content: message.content,
                })
                .collect(),
            task_id: None,
            max_tokens: Some(max_tokens),
            temperature: Some(temperature),
            stream: Some(true),
        };

        let mut stream = client.chat(request).await.map_err(|err| map_status(&err))?;

        Ok(Box::pin(async_stream::stream! {
            use tokio_stream::StreamExt as _;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(chunk) => {
                        if chunk.done {
                            break;
                        }
                        if !chunk.delta.is_empty() {
                            yield Ok(chunk.delta);
                        }
                    }
                    Err(status) => {
                        yield Err(map_status(&status));
                        break;
                    }
                }
            }
        }))
    }
}

/// Map a transport error onto the closest HTTP meaning.
///
/// Takes anything printable because the two failure sources - a failed call and
/// an error item inside a live stream - surface as different types, and a client
/// should not have to care which one it got.
fn map_status(status: &dyn std::fmt::Display) -> ApiError {
    let text = status.to_string();
    if text.contains("not found") || text.contains("not a registered model") {
        ApiError::ModelNotFound(text)
    } else if text.contains("resource exhausted") {
        ApiError::Backend("another model is loaded; retry when it finishes".to_owned())
    } else if text.contains("unimplemented") {
        ApiError::CoreUnavailable("no runtime is installed".to_owned())
    } else {
        ApiError::Backend(text)
    }
}
