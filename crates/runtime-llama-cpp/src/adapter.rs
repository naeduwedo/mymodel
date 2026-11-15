//! The llama.cpp RuntimeAdapter implementation (spec 8.3).
//!
//! The adapter owns the whole lifecycle: probe the binary, report capabilities,
//! estimate memory, spawn the worker, stream chat, and unload. Every capability
//! it reports comes from what the probe actually found on this machine.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use gguf_engine_hardware::ComputeBackend;
use gguf_engine_runtime_manager::{
    Capability, CapabilityStatus, ChatRequest, LoadPlan, MemoryEstimate, RuntimeAdapter,
    RuntimeCapabilities, RuntimeError, RuntimeInstance, TokenStream,
};

use crate::client;
use crate::probe::{ProbeResult, locate};
use crate::process::{self, Worker};

/// The adapter, holding the probe result and the live instances.
#[derive(Debug)]
pub struct LlamaCppAdapter {
    probe: ProbeResult,
    instances: Arc<RwLock<HashMap<String, Worker>>>,
}

impl LlamaCppAdapter {
    /// Locate and probe the installed llama.cpp.
    pub fn discover() -> Result<Self, RuntimeError> {
        Ok(Self {
            probe: locate()?,
            instances: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Build from an already-known probe result (tests).
    pub fn from_probe(probe: ProbeResult) -> Self {
        Self {
            probe,
            instances: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// The binary directory, for display.
    pub fn directory(&self) -> String {
        self.probe.directory.display().to_string()
    }
}

#[async_trait]
impl RuntimeAdapter for LlamaCppAdapter {
    fn id(&self) -> &'static str {
        "llama.cpp"
    }

    async fn probe(&self) -> Result<CapabilityStatus, RuntimeError> {
        Ok(CapabilityStatus::Supported)
    }

    async fn capabilities(&self) -> Result<RuntimeCapabilities, RuntimeError> {
        let cuda = self.probe.backends.contains(&ComputeBackend::Cuda);
        let mut caps = RuntimeCapabilities {
            modalities: vec!["text".to_owned()],
            architectures: gguf_engine_gguf::KNOWN_ARCHITECTURES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            backends: self.probe.backends.clone(),
            supports_multi_gpu: cuda && self.probe.cuda_devices > 1,
            supports_gpu_offload: cuda,
            supports_mmap: true,
            supports_streaming: true,
            capabilities: Vec::new(),
        };

        caps.capabilities.push(Capability {
            name: "multi_gpu".to_owned(),
            status: if caps.supports_multi_gpu {
                CapabilityStatus::Supported
            } else {
                CapabilityStatus::Unsupported
            },
            detail: (!caps.supports_multi_gpu)
                .then(|| format!("{} CUDA device(s) detected", self.probe.cuda_devices)),
        });
        caps.capabilities.push(Capability {
            name: "gpu_offload".to_owned(),
            status: if cuda {
                CapabilityStatus::Supported
            } else {
                CapabilityStatus::Unsupported
            },
            detail: None,
        });
        caps.capabilities.push(Capability {
            name: "streaming".to_owned(),
            status: CapabilityStatus::Supported,
            detail: None,
        });
        caps.capabilities.push(Capability {
            name: "engine_managed_paging".to_owned(),
            status: CapabilityStatus::Unsupported,
            detail: Some(
                "llama.cpp manages its own memory; the engine must not move tensors".to_owned(),
            ),
        });

        Ok(caps)
    }

    async fn estimate_memory(&self, plan: &LoadPlan) -> Result<MemoryEstimate, RuntimeError> {
        let file_len = std::fs::metadata(&plan.model_path)
            .map(|m| m.len())
            .unwrap_or(0);
        let kv_cache_bytes = u64::from(plan.context_size) * 1024;
        let workspace_bytes = 64 * 1024 * 1024;
        Ok(MemoryEstimate {
            weights_bytes: file_len,
            kv_cache_bytes,
            workspace_bytes,
            total_bytes: file_len
                .saturating_add(kv_cache_bytes)
                .saturating_add(workspace_bytes),
            confidence: CapabilityStatus::Experimental,
        })
    }

    async fn load(&self, plan: &LoadPlan) -> Result<RuntimeInstance, RuntimeError> {
        let port = pick_port().await?;
        let instance_id = uuid::Uuid::now_v7().to_string();
        let stderr_file = std::env::temp_dir().join(format!("gguf-llama-{instance_id}.log"));

        let worker = process::spawn(
            &self.probe.server_path,
            std::path::Path::new(&plan.model_path),
            port,
            plan.gpu_layers.unwrap_or(0),
            plan.context_size.max(128),
            plan.mode == gguf_engine_runtime_manager::LoadMode::Embeddings,
            &stderr_file,
        )
        .await?;

        let pid = worker.pid();
        self.instances.write().insert(instance_id.clone(), worker);

        Ok(RuntimeInstance {
            instance_id,
            runtime_id: self.id().to_owned(),
            model_id: plan.model_path.clone(),
            worker_pid: pid,
            endpoint: Some(format!("http://127.0.0.1:{port}")),
        })
    }

    async fn chat(
        &self,
        instance: &RuntimeInstance,
        request: &ChatRequest,
    ) -> Result<TokenStream, RuntimeError> {
        let base = instance
            .endpoint
            .clone()
            .ok_or_else(|| RuntimeError::Start("instance has no endpoint".to_owned()))?;
        Ok(client::stream_completion(
            base,
            request.messages.clone(),
            request.max_tokens.max(1),
            request.temperature,
        ))
    }

    async fn unload(&self, instance: RuntimeInstance) -> Result<(), RuntimeError> {
        // Take the worker out of the map and drop the lock guard BEFORE
        // awaiting shutdown; a parking_lot guard held across an await makes the
        // future !Send and breaks async_trait's Send requirement.
        let worker = self.instances.write().remove(&instance.instance_id);
        if let Some(worker) = worker {
            process::shutdown(worker).await?;
        }
        Ok(())
    }
}

/// Bind a free loopback port and return it.
async fn pick_port() -> Result<u16, RuntimeError> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| RuntimeError::Start(e.to_string()))?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| RuntimeError::Start(e.to_string()))
}
