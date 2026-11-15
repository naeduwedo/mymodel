//! The planner (spec 5.5): pick a strategy from what the hardware and runtime allow.
//!
//! The planner is a pure function of (devices, budgets, requirement, policy) so it
//! can be tested exhaustively without a GPU. Its overriding rule is that it only
//! proposes what is possible: an unsupported strategy is never returned as a
//! fallback that will fail at load time.

use gguf_engine_config::GpuConfig;
use gguf_engine_hardware::{GpuDevice, GpuVendor};

use crate::types::{
    ExecutionPlan, ExecutionStrategy, GpuBudget, ModelMemoryRequirement, PlanError,
};

/// Runtime facts the planner needs. Nothing here is inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeFacts {
    /// Runtime can offload layers to a GPU.
    pub supports_gpu_offload: bool,
    /// Runtime declares (and has validated) tensor parallel support.
    pub supports_tensor_parallel: bool,
    /// Runtime can load the model with partially offloaded layers.
    pub supports_cpu_offload: bool,
}

/// Total usable VRAM across devices that report a discrete pool.
pub fn total_usable(budgets: &[GpuBudget]) -> u64 {
    budgets.iter().map(|budget| budget.usable).sum()
}

/// Plan a load.
///
/// Order of preference: a single GPU that fits (stability first), then a layer
/// split across same-vendor devices, then CPU offload. Tensor parallel is only
/// considered when the runtime declares it and two devices of the same vendor
/// are present, because it is the strategy most likely to fail in practice.
pub fn plan(
    devices: &[GpuDevice],
    budgets: &[GpuBudget],
    requirement: &ModelMemoryRequirement,
    policy: &GpuConfig,
    runtime: RuntimeFacts,
) -> Result<ExecutionPlan, PlanError> {
    let need = requirement.total();

    if devices.is_empty() {
        return Err(PlanError {
            reason: "no GPU was detected; CPU-only inference is not implemented yet".to_owned(),
        });
    }

    if !runtime.supports_gpu_offload {
        return Err(PlanError {
            reason: "the installed runtime cannot offload layers to a GPU".to_owned(),
        });
    }

    // Budgets are keyed by device id; align them with the device order.
    let budget_of = |id: &str| -> u64 {
        budgets
            .iter()
            .find(|budget| budget.device_id == id)
            .map(|budget| budget.usable)
            .unwrap_or(0)
    };

    // 1. A single device that fits.
    let mut single: Vec<(&GpuDevice, u64)> = devices
        .iter()
        .map(|device| (device, budget_of(&device.id)))
        .collect();
    single.sort_by(|a, b| b.1.cmp(&a.1));

    if let Some((device, usable)) = single.first()
        && *usable >= need
    {
        return Ok(ExecutionPlan {
            strategy: ExecutionStrategy::SingleGpu,
            devices: vec![device.id.clone()],
            reserved_bytes: vec![need],
            reason: format!(
                "all {:.1} GiB fit on {} ({:.1} GiB usable)",
                gib(need),
                device.name,
                gib(*usable)
            ),
        });
    }

    // 2. Multi-GPU is only reachable when policy allows it. Not a hard error:
    //    CPU offload needs a single device and may still fit the request, so the
    //    planner records why multi-GPU was skipped and continues.
    let largest_usable = single.first().map(|(_, usable)| *usable).unwrap_or(0);
    let mut multi_gpu_blocked: Option<String> = None;
    if !policy.allow_multi_gpu {
        multi_gpu_blocked = Some(format!(
            "needs {:.1} GiB but the largest GPU offers {:.1} GiB usable, and multi-GPU is disabled",
            gib(need),
            gib(largest_usable)
        ));
    }

    // 3. Tensor parallel: only with runtime support and same-vendor devices.
    let same_vendor_pair = devices.iter().any(|a| {
        devices.iter().any(|b| {
            a.id != b.id
                && a.vendor == b.vendor
                && a.vendor != GpuVendor::Unknown
                && budget_of(&a.id) > 0
                && budget_of(&b.id) > 0
        })
    });
    // Compare vendors via their stable names: GpuVendor has no Ord, and the set
    // only needs to know whether more than one vendor is present.
    let mut vendors: Vec<&'static str> = devices.iter().map(|d| d.vendor.as_str()).collect();
    vendors.sort_unstable();
    vendors.dedup();
    let cross_vendor = vendors.len() > 1;

    if cross_vendor && !policy.allow_cross_vendor_model_split {
        multi_gpu_blocked = Some(format!(
            "needs {:.1} GiB; the GPUs are from different vendors and cross-vendor splitting is disabled",
            gib(need)
        ));
    }

    let usable_total = total_usable(budgets);
    if multi_gpu_blocked.is_none() && usable_total >= need {
        let strategy = if runtime.supports_tensor_parallel && same_vendor_pair {
            ExecutionStrategy::TensorParallel
        } else {
            ExecutionStrategy::LayerSplit
        };

        // Spread the requirement proportionally to each device's usable VRAM.
        let mut devices_used = Vec::new();
        let mut reserved = Vec::new();
        let mut remaining = need;
        for (device, usable) in single.iter() {
            if *usable == 0 || remaining == 0 {
                continue;
            }
            let share = (*usable).min(remaining);
            devices_used.push(device.id.clone());
            reserved.push(share);
            remaining -= share;
        }

        return Ok(ExecutionPlan {
            strategy,
            devices: devices_used,
            reserved_bytes: reserved,
            reason: format!(
                "{:.1} GiB across {} GPUs ({})",
                gib(need),
                single.len(),
                strategy.as_str()
            ),
        });
    }

    // 4. CPU offload: the runtime keeps what it can on the GPU.
    if runtime.supports_cpu_offload {
        let (device, usable) = single.first().copied().unwrap_or((&devices[0], 0));
        return Ok(ExecutionPlan {
            strategy: ExecutionStrategy::CpuOffload,
            devices: vec![device.id.clone()],
            reserved_bytes: vec![usable],
            reason: format!(
                "needs {:.1} GiB but only {:.1} GiB of VRAM is usable; the rest stays in RAM (slower)",
                gib(need),
                gib(usable_total)
            ),
        });
    }

    let reason = match multi_gpu_blocked {
        Some(blocked) => format!(
            "{blocked}; the runtime cannot offload to RAM either, so this model cannot run here"
        ),
        None => format!(
            "needs {:.1} GiB but only {:.1} GiB of usable VRAM is available, and the runtime cannot offload to RAM",
            gib(need),
            gib(usable_total)
        ),
    };
    Err(PlanError { reason })
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}
