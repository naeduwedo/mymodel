//! `InferenceService`: streaming chat over the local transport (spec §9).
//!
//! This is the boundary the UI shells call. It owns three responsibilities and
//! nothing else:
//!
//! 1. **Resolve** a `model_id` to a file on disk - a direct path, or a
//!    registered repository's newest revision;
//! 2. **Load** it through the runtime adapter and **unload** it when the stream
//!    ends, including when the client disconnects mid-answer;
//! 3. **Stream** tokens as they are generated.
//!
//! Concurrency is deliberately limited to one loaded model. A second request is
//! refused with `RESOURCE_EXHAUSTED` rather than queued, because queueing would
//! leave a user staring at a spinner with no explanation while their VRAM is
//! already committed to someone else's model.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use gguf_engine_protocol::pb;
use gguf_engine_runtime_llama_cpp::LlamaCppAdapter;
use gguf_engine_runtime_manager::{
    ChatMessage, ChatRequest as AdapterChatRequest, LoadMode, LoadPlan, RuntimeAdapter,
    RuntimeInstance,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};
use tracing::{debug, info, warn};

use crate::state::CoreState;

/// Context window used when the client does not specify one.
const DEFAULT_CONTEXT: u32 = 4096;

/// `InferenceService` implementation.
#[derive(Debug)]
pub struct InferenceService {
    state: CoreState,
    adapter: Option<Arc<LlamaCppAdapter>>,
    /// One permit: one loaded model at a time.
    slots: Arc<Semaphore>,
}

impl InferenceService {
    /// Create the service, probing the installed runtime.
    pub fn new(state: CoreState) -> Self {
        let adapter = LlamaCppAdapter::discover().ok().map(Arc::new);
        Self {
            state,
            adapter,
            slots: Arc::new(Semaphore::new(1)),
        }
    }

    /// Resolve a `model_id` to a model file.
    ///
    /// A path that exists on disk is accepted directly: that keeps the service
    /// usable from tests and from a caller that already knows the file. Anything
    /// else is treated as a registered repository id.
    async fn resolve_model(&self, model_id: &str) -> Result<PathBuf, Status> {
        let direct = Path::new(model_id);
        if direct.is_file() {
            return Ok(direct.to_path_buf());
        }

        let revisions = self
            .state
            .database
            .list_revisions()
            .await
            .map_err(|err| Status::internal(format!("reading the model registry: {err}")))?;

        let newest = revisions
            .iter()
            .filter(|record| record.model_id == model_id)
            .max_by_key(|record| record.recorded_at);

        let Some(newest) = newest else {
            return Err(Status::not_found(format!(
                "'{model_id}' is neither a file on disk nor a registered model; \
                 run `gguf models pull` first"
            )));
        };

        let record = self
            .state
            .database
            .load_revision(&newest.model_id, &newest.revision)
            .await
            .map_err(|err| Status::internal(format!("reading the model registry: {err}")))?
            .ok_or_else(|| {
                Status::not_found(format!(
                    "revision {}@{} disappeared from the registry",
                    newest.model_id, newest.revision
                ))
            })?;

        let weights = record
            .files
            .iter()
            .find(|file| file.role == "weights")
            .ok_or_else(|| {
                Status::failed_precondition(format!(
                    "{model_id} has no weights file recorded; it was never fully downloaded"
                ))
            })?;

        let path = Path::new(&self.state.models_dir).join(&weights.path);
        if !path.is_file() {
            return Err(Status::failed_precondition(format!(
                "{model_id} is registered but the file is missing: {}",
                path.display()
            )));
        }

        Ok(path)
    }
}

/// Unload a model when the stream ends for any reason.
///
/// `Drop` runs on normal completion *and* when a client disconnects, which is
/// exactly when a naive `unload()` at the end of the generator would be skipped,
/// leaving a llama-server process holding VRAM until the core restarted.
struct UnloadGuard {
    adapter: Arc<LlamaCppAdapter>,
    instance: Option<RuntimeInstance>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for UnloadGuard {
    fn drop(&mut self) {
        let Some(instance) = self.instance.take() else {
            return;
        };
        let adapter = self.adapter.clone();
        // Drop cannot await, so the unload is handed to the runtime. The worker
        // also carries kill_on_drop, so even a lost task cannot leak a process.
        tokio::spawn(async move {
            if let Err(err) = adapter.unload(instance).await {
                warn!(error = %err, "unloading the model after the stream ended failed");
            }
        });
    }
}

#[tonic::async_trait]
impl pb::inference_service_server::InferenceService for InferenceService {
    type ChatStream = Pin<Box<dyn Stream<Item = Result<pb::ChatChunk, Status>> + Send>>;

    async fn embeddings(
        &self,
        request: Request<pb::EmbeddingsRequest>,
    ) -> Result<Response<pb::EmbeddingsResponse>, Status> {
        let request = request.into_inner();

        let Some(adapter) = self.adapter.clone() else {
            return Err(Status::unimplemented(
                "no runtime is installed: install llama.cpp and set GGUF_LLAMA_CPP_DIR",
            ));
        };

        if request.input.is_empty() {
            return Err(Status::invalid_argument("input must not be empty"));
        }
        if request.model_id.trim().is_empty() {
            return Err(Status::invalid_argument("model_id must not be empty"));
        }

        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            Status::resource_exhausted("another model is loaded; wait for that answer to finish")
        })?;

        let model_path = self.resolve_model(&request.model_id).await?;

        // Embeddings need a worker started in embedding mode; there is no
        // request-time switch, which is why the mode is part of the load plan.
        let plan = LoadPlan {
            model_path: model_path.display().to_string(),
            gpu_layers: None,
            context_size: DEFAULT_CONTEXT,
            mode: LoadMode::Embeddings,
        };

        let instance = adapter.load(&plan).await.map_err(|err| {
            warn!(error = %err, "loading the model for embeddings failed");
            Status::failed_precondition(format!(
                "this model cannot be loaded in embedding mode: {err}"
            ))
        })?;

        let Some(endpoint) = instance.endpoint.clone() else {
            let _ = adapter.unload(instance).await;
            drop(permit);
            return Err(Status::internal("the worker did not report an endpoint"));
        };

        let vectors =
            gguf_engine_runtime_llama_cpp::client::embeddings(&endpoint, &request.input).await;

        // Unload before returning, whatever happened: an embedding call leaves no
        // model resident, because the next request may need a different one.
        let unload = adapter.unload(instance).await;
        drop(permit);
        if let Err(err) = unload {
            warn!(error = %err, "unloading after an embedding request failed");
        }

        let vectors = vectors.map_err(|err| {
            Status::failed_precondition(format!("the runtime refused the embedding request: {err}"))
        })?;

        let dimensions = vectors.first().map(|vector| vector.len()).unwrap_or(0);
        let embeddings: Vec<pb::Embedding> = vectors
            .into_iter()
            .map(|values| pb::Embedding { values })
            .collect();

        Ok(Response::new(pb::EmbeddingsResponse {
            model_id: request.model_id,
            embeddings,
            dimensions: dimensions as u32,
        }))
    }

    async fn chat(
        &self,
        request: Request<pb::ChatRequest>,
    ) -> Result<Response<Self::ChatStream>, Status> {
        let request = request.into_inner();

        let Some(adapter) = self.adapter.clone() else {
            return Err(Status::unimplemented(
                "no runtime is installed: install llama.cpp and set GGUF_LLAMA_CPP_DIR",
            ));
        };

        if request.messages.is_empty() {
            return Err(Status::invalid_argument("messages must not be empty"));
        }
        if request.model_id.trim().is_empty() {
            return Err(Status::invalid_argument("model_id must not be empty"));
        }

        // Refuse rather than queue: a queued request looks like a hang, and the
        // VRAM is already committed to whoever holds the slot.
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            Status::resource_exhausted("another model is loaded; wait for that answer to finish")
        })?;

        let model_path = self.resolve_model(&request.model_id).await?;
        debug!(model = %model_path.display(), "loading for a chat request");

        let plan = LoadPlan {
            model_path: model_path.display().to_string(),
            gpu_layers: None,
            context_size: DEFAULT_CONTEXT,
            mode: LoadMode::Chat,
        };

        let instance = adapter.load(&plan).await.map_err(|err| {
            warn!(error = %err, "loading the model failed");
            Status::internal(format!("loading the model: {err}"))
        })?;

        info!(
            instance = %instance.instance_id,
            pid = ?instance.worker_pid,
            "model loaded for a chat request"
        );

        let messages: Vec<ChatMessage> = request
            .messages
            .iter()
            .map(|message| ChatMessage {
                role: message.role.clone(),
                content: message.content.clone(),
            })
            .collect();

        let adapter_request = AdapterChatRequest {
            messages,
            max_tokens: request.max_tokens.unwrap_or(512).max(1),
            temperature: request.temperature.unwrap_or(0.7),
            stream: true,
        };

        let tokens = adapter
            .chat(&instance, &adapter_request)
            .await
            .map_err(|err| Status::internal(format!("starting inference: {err}")))?;

        let task_id = request
            .task_id
            .unwrap_or_else(|| instance.instance_id.clone());

        // The guard owns the instance from here on, so every exit path unloads.
        let guard = UnloadGuard {
            adapter: adapter.clone(),
            instance: Some(instance),
            _permit: permit,
        };

        let started = Instant::now();
        let mut tokens = tokens;

        let stream = async_stream::stream! {
            // Bind the guard so it lives exactly as long as the stream.
            let _guard = guard;
            let mut generated: u32 = 0;
            let mut failed = false;

            while let Some(item) = tokens.next().await {
                match item {
                    Ok(token) => {
                        generated += 1;
                        yield Ok(pb::ChatChunk {
                            task_id: task_id.clone(),
                            delta: token.content,
                            done: false,
                            progress: Some(progress(
                                pb::TaskStatus::Running,
                                generated,
                                started.elapsed().as_millis() as u64,
                            )),
                        });
                    }
                    Err(err) => {
                        failed = true;
                        yield Err(Status::internal(format!("generating tokens: {err}")));
                        break;
                    }
                }
            }

            if !failed {
                let elapsed = started.elapsed().as_millis() as u64;
                yield Ok(pb::ChatChunk {
                    task_id,
                    delta: String::new(),
                    done: true,
                    progress: Some(progress(
                        pb::TaskStatus::Completed,
                        generated,
                        elapsed,
                    )),
                });
            }
        };

        Ok(Response::new(Box::pin(stream) as Self::ChatStream))
    }
}

fn progress(status: pb::TaskStatus, generated: u32, elapsed_ms: u64) -> pb::TaskProgress {
    pb::TaskProgress {
        status: status as i32,
        tokens_generated: generated,
        elapsed_ms,
        // Tokens per second is only meaningful once something has been generated
        // and time has passed; reporting 0.0 for an empty answer would be a lie.
        tokens_per_second: (generated > 0 && elapsed_ms > 0)
            .then(|| f64::from(generated) * 1000.0 / elapsed_ms as f64),
    }
}
