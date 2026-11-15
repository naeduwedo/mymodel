//! Runtime adapter types (spec section 8).
//!
//! The capability status is four-state, never a boolean: supported,
//! experimental, unsupported, unknown. A probe that could not answer is
//! unknown, and a UI must show it as such.

use gguf_engine_hardware::ComputeBackend;
use serde::{Deserialize, Serialize};

/// Whether a runtime capability is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// Verified on this machine.
    Supported,
    /// Present but not validated against this model or device.
    Experimental,
    /// The runtime does not provide this.
    Unsupported,
    /// Could not be determined.
    Unknown,
}

impl CapabilityStatus {
    /// Stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Experimental => "experimental",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }
}

/// A named capability and its status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    /// Stable id, e.g. multi_gpu, mmap, streaming.
    pub name: String,
    /// Status.
    pub status: CapabilityStatus,
    /// Explanation when not supported.
    pub detail: Option<String>,
}

/// What a runtime advertises it can do (spec section 8.1).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RuntimeCapabilities {
    /// Modalities (text, vision, ...).
    pub modalities: Vec<String>,
    /// Model architectures it can load.
    pub architectures: Vec<String>,
    /// Compute backends it can use.
    pub backends: Vec<ComputeBackend>,
    /// Whether it can split one model across GPUs.
    pub supports_multi_gpu: bool,
    /// Whether it can offload layers to a GPU.
    pub supports_gpu_offload: bool,
    /// Whether it memory-maps model files.
    pub supports_mmap: bool,
    /// Whether it streams tokens as they are generated.
    pub supports_streaming: bool,
    /// Per-capability status.
    pub capabilities: Vec<Capability>,
}

/// A running runtime instance: one loaded model in one worker process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInstance {
    /// Unique id.
    pub instance_id: String,
    /// Runtime adapter id.
    pub runtime_id: String,
    /// The loaded model.
    pub model_id: String,
    /// Worker process id.
    pub worker_pid: Option<u32>,
    /// Endpoint the worker serves.
    pub endpoint: Option<String>,
}

/// A chunk of generated text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    /// Text of this chunk.
    pub content: String,
}

/// A single chat message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// system, user or assistant.
    pub role: String,
    /// Message content.
    pub content: String,
}

/// A chat completion request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Conversation history.
    pub messages: Vec<ChatMessage>,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// Whether to stream tokens.
    pub stream: bool,
}

/// Why a runtime operation failed.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The runtime binary could not be found.
    #[error("runtime binary not found: {0}")]
    NotFound(String),
    /// The binary exists but could not be probed.
    #[error("probing runtime {runtime_id}: {reason}")]
    Probe {
        /// Runtime id.
        runtime_id: String,
        /// Why the probe failed.
        reason: String,
    },
    /// A worker process failed to start.
    #[error("worker failed to start: {0}")]
    Start(String),
    /// A worker exited unexpectedly.
    #[error("worker exited (code {code:?}): {stderr}")]
    WorkerExited {
        /// Exit code, when known.
        code: Option<i32>,
        /// Last stderr lines, for diagnostics.
        stderr: String,
    },
    /// The worker never became ready.
    #[error("worker did not become ready within {timeout_ms} ms")]
    NotReady {
        /// How long we waited.
        timeout_ms: u64,
    },
    /// The runtime HTTP endpoint rejected the request.
    #[error("runtime HTTP error: {0}")]
    Http(String),
    /// The operation was cancelled.
    #[error("cancelled")]
    Cancelled,
}
