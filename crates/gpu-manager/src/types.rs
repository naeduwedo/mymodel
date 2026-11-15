//! GPU scheduling types (spec 5.1, 5.2, 5.4, 5.6).

use gguf_engine_hardware::GpuDevice;
use serde::{Deserialize, Serialize};

/// How one model may be spread across devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStrategy {
    /// One model bound to one GPU.
    SingleGpu,
    /// Different GPUs hold different layers of the same model.
    LayerSplit,
    /// Multiple GPUs compute the same layer together.
    TensorParallel,
    /// Different GPUs run different models or tasks.
    TaskParallel,
    /// Some weights stay in host RAM.
    CpuOffload,
    /// GPU plus RAM plus runtime NVMe access.
    HybridOffload,
}

impl ExecutionStrategy {
    /// Stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SingleGpu => "single_gpu",
            Self::LayerSplit => "layer_split",
            Self::TensorParallel => "tensor_parallel",
            Self::TaskParallel => "task_parallel",
            Self::CpuOffload => "cpu_offload",
            Self::HybridOffload => "hybrid_offload",
        }
    }
}

/// How much memory one model load needs (spec 5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModelMemoryRequirement {
    /// Weight bytes.
    pub weights: u64,
    /// KV cache bytes.
    pub kv_cache: u64,
    /// Activation bytes.
    pub activations: u64,
    /// Compute workspace bytes.
    pub workspace: u64,
}

impl ModelMemoryRequirement {
    /// Everything except the KV cache.
    pub fn fixed_bytes(&self) -> u64 {
        self.weights
            .saturating_add(self.activations)
            .saturating_add(self.workspace)
    }

    /// Total bytes.
    pub fn total(&self) -> u64 {
        self.fixed_bytes().saturating_add(self.kv_cache)
    }
}

/// The usable VRAM budget of one device (spec 5.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuBudget {
    /// Device id.
    pub device_id: String,
    /// Free VRAM the probe observed.
    pub observed_free: u64,
    /// Engine reserve kept free.
    pub engine_reserve: u64,
    /// Safety margin on top of the reserve.
    pub safety_margin: u64,
    /// What remains for model allocations.
    pub usable: u64,
}

impl GpuBudget {
    /// Compute a budget. Unknown free VRAM yields zero usable bytes,
    /// because scheduling against an unknown amount is scheduling nothing.
    pub fn for_device(device: &GpuDevice, engine_reserve: u64, safety_margin: u64) -> Self {
        let observed_free = device.free_vram_bytes.unwrap_or(0);
        let usable = observed_free
            .saturating_sub(engine_reserve)
            .saturating_sub(safety_margin);
        Self {
            device_id: device.id.clone(),
            observed_free,
            engine_reserve,
            safety_margin,
            usable,
        }
    }
}

/// A plan for loading one model (spec 5.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    /// Strategy chosen.
    pub strategy: ExecutionStrategy,
    /// Device ids the plan uses.
    pub devices: Vec<String>,
    /// VRAM reserved on each device, matching devices order.
    pub reserved_bytes: Vec<u64>,
    /// Explanation for the user.
    pub reason: String,
}

/// Why no plan could be made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanError {
    /// Reason, actionable for the user.
    pub reason: String,
}
