//! The runtime adapter trait (spec section 8.2).
//!
//! Every runtime — llama.cpp first, image/video/audio later — implements this
//! one boundary. The responsibilities are deliberately fixed: probe,
//! capabilities, memory estimate, load, infer, unload. A runtime that cannot
//! estimate memory, or that has no streaming, says so through its capability
//! report rather than by pretending.

use async_trait::async_trait;
use tokio_stream::Stream;

use crate::types::{
    CapabilityStatus, ChatRequest, RuntimeCapabilities, RuntimeError, RuntimeInstance, Token,
};

/// A stream of generated tokens.
pub type TokenStream = std::pin::Pin<Box<dyn Stream<Item = Result<Token, RuntimeError>> + Send>>;

/// A memory estimate for one model load (spec section 8.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEstimate {
    /// Estimated weights bytes.
    pub weights_bytes: u64,
    /// Estimated KV cache bytes at the requested context.
    pub kv_cache_bytes: u64,
    /// Estimated activation/workspace bytes.
    pub workspace_bytes: u64,
    /// Total of the above.
    pub total_bytes: u64,
    /// Whether the estimate is trustworthy or a rough bound.
    pub confidence: CapabilityStatus,
}

impl MemoryEstimate {
    /// Total bytes.
    pub fn total(&self) -> u64 {
        self.total_bytes
    }
}

/// What the loaded worker will be used for (spec section 8.2).
///
/// llama.cpp needs to know at startup: a worker started for chat cannot answer an
/// embedding request, and one started with `--embeddings` refuses to chat. Making
/// the mode part of the plan is what keeps the failure at load time, where it can
/// explain itself, instead of at the first request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadMode {
    /// Text generation (the default).
    #[default]
    Chat,
    /// Vector embeddings.
    Embeddings,
}

/// A load plan: what to load, onto which devices (spec section 5.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadPlan {
    /// Path to the model file.
    pub model_path: String,
    /// Layers to offload to the GPU; None = let the runtime decide.
    pub gpu_layers: Option<u32>,
    /// Context size in tokens.
    pub context_size: u32,
    /// What the worker will be used for.
    #[serde(default)]
    pub mode: LoadMode,
}

/// The common boundary every runtime adapter implements.
#[async_trait]
pub trait RuntimeAdapter: Send + Sync {
    /// Stable runtime id, e.g. llama.cpp.
    fn id(&self) -> &'static str;

    /// Probe the installed runtime and report whether it is usable.
    async fn probe(&self) -> Result<CapabilityStatus, RuntimeError>;

    /// What this runtime can do on this machine.
    async fn capabilities(&self) -> Result<RuntimeCapabilities, RuntimeError>;

    /// Estimate memory for a load plan. Honest about uncertainty.
    async fn estimate_memory(&self, plan: &LoadPlan) -> Result<MemoryEstimate, RuntimeError>;

    /// Load a model, returning a live instance.
    async fn load(&self, plan: &LoadPlan) -> Result<RuntimeInstance, RuntimeError>;

    /// Run a chat completion, returning a token stream.
    async fn chat(
        &self,
        instance: &RuntimeInstance,
        request: &ChatRequest,
    ) -> Result<TokenStream, RuntimeError>;

    /// Unload a model, releasing the worker process.
    async fn unload(&self, instance: RuntimeInstance) -> Result<(), RuntimeError>;
}

// Re-export serde so the derive above resolves without a direct dep in users.
#[doc(hidden)]
pub use serde::{Deserialize, Serialize};

/// Shorthand for a boxed adapter.
pub type BoxedAdapter = std::sync::Arc<dyn RuntimeAdapter>;
