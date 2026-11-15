//! `gguf plan` - show how a model would be scheduled on this machine.
//!
//! Reads the real hardware inventory, computes per-device VRAM budgets from the
//! configured reserves, and asks the planner for a strategy. Nothing is loaded;
//! this answers "would this fit, and how" before a user downloads 40 GB.

use anyhow::Result;
use gguf_engine_config::GpuConfig;
use gguf_engine_gpu_manager::{
    GpuBudget, ModelMemoryRequirement, RuntimeFacts, plan, total_usable,
};
use gguf_engine_hardware::collect;
use serde::Serialize;

use crate::commands::Context;
use crate::output;

#[derive(Debug, Serialize)]
struct BudgetRow {
    device: String,
    name: String,
    observed_free: u64,
    engine_reserve: u64,
    safety_margin: u64,
    usable: u64,
}

#[derive(Debug, Serialize)]
struct PlanReport {
    ok: bool,
    strategy: String,
    reason: String,
    devices: Vec<String>,
    reserved_bytes: Vec<u64>,
    total_usable: u64,
    budgets: Vec<BudgetRow>,
}

/// Plan a load of `weights_gib` + `kv_gib`.
pub fn plan_cmd(
    ctx: &Context,
    weights_gib: f64,
    kv_gib: f64,
    context_size: u32,
    json: bool,
) -> Result<bool> {
    let snapshot = collect();

    if snapshot.gpus.value.is_empty() {
        let reason = snapshot
            .gpus
            .reason
            .clone()
            .unwrap_or_else(|| "no GPU was detected".to_owned());
        if json {
            println!("{}", serde_json::json!({ "ok": false, "reason": reason }));
        } else {
            println!("cannot plan: {reason}");
        }
        return Ok(false);
    }

    let memory = &ctx.config.memory;
    let budgets: Vec<GpuBudget> = snapshot
        .gpus
        .value
        .iter()
        .map(|device| {
            GpuBudget::for_device(
                device,
                memory.gpu_reserve_bytes,
                memory.gpu_safety_margin_bytes,
            )
        })
        .collect();

    // KV cache scales with context; the caller supplies the weights.
    let requirement = ModelMemoryRequirement {
        weights: gib_to_bytes(weights_gib),
        kv_cache: gib_to_bytes(kv_gib),
        activations: 512 * 1024 * 1024,
        workspace: 256 * 1024 * 1024,
    };

    // llama.cpp facts, as probed by the adapter in Task 6.
    let runtime = RuntimeFacts {
        supports_gpu_offload: true,
        supports_tensor_parallel: false,
        supports_cpu_offload: true,
    };

    let policy: GpuConfig = ctx.config.gpu.clone();
    let rows: Vec<BudgetRow> = snapshot
        .gpus
        .value
        .iter()
        .zip(budgets.iter())
        .map(|(device, budget)| BudgetRow {
            device: device.id.clone(),
            name: device.name.clone(),
            observed_free: budget.observed_free,
            engine_reserve: budget.engine_reserve,
            safety_margin: budget.safety_margin,
            usable: budget.usable,
        })
        .collect();

    match plan(
        &snapshot.gpus.value,
        &budgets,
        &requirement,
        &policy,
        runtime,
    ) {
        Ok(execution) => {
            let report = PlanReport {
                ok: true,
                strategy: execution.strategy.as_str().to_owned(),
                reason: execution.reason,
                devices: execution.devices,
                reserved_bytes: execution.reserved_bytes,
                total_usable: total_usable(&budgets),
                budgets: rows,
            };

            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
                return Ok(true);
            }

            print_budgets(&report);
            println!();
            println!("strategy : {}", report.strategy);
            println!("reason   : {}", report.reason);
            for (device, bytes) in report.devices.iter().zip(report.reserved_bytes.iter()) {
                println!("  {device} <- {}", output::human_bytes(*bytes));
            }
            let _ = context_size;
            Ok(true)
        }
        Err(err) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "ok": false, "reason": err.reason })
                );
            } else {
                print_budgets(&PlanReport {
                    ok: false,
                    strategy: String::new(),
                    reason: err.reason.clone(),
                    devices: Vec::new(),
                    reserved_bytes: Vec::new(),
                    total_usable: total_usable(&budgets),
                    budgets: rows,
                });
                println!();
                println!("cannot plan: {}", err.reason);
            }
            Ok(false)
        }
    }
}

fn print_budgets(report: &PlanReport) {
    println!("GPU BUDGETS");
    for row in &report.budgets {
        println!(
            "  {:<24} usable {:>10} of {:>10} free (reserve {} + margin {})",
            row.name,
            output::human_bytes(row.usable),
            output::human_bytes(row.observed_free),
            output::human_bytes(row.engine_reserve),
            output::human_bytes(row.safety_margin)
        );
    }
    println!(
        "  {:<24} total usable {:>10} (never treated as one pool)",
        "",
        output::human_bytes(report.total_usable)
    );
}

fn gib_to_bytes(gib: f64) -> u64 {
    (gib * 1024.0 * 1024.0 * 1024.0) as u64
}
