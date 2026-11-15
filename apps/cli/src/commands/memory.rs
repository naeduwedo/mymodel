//! `gguf memory` - the three-tier memory picture (spec 6).
//!
//! Prints VRAM per device, host RAM and NVMe, each with its reserve and usable
//! figure, plus the current pressure classification. NVMe is shown as a cold
//! tier and is never presented as usable memory.

use anyhow::Result;
use gguf_engine_hardware::collect;
use gguf_engine_memory_manager::{
    PrefetchRequest, Pressure, Tier, TierBudget, grant_prefetch, host_memory,
};
use serde::Serialize;

use crate::commands::Context;
use crate::output;

#[derive(Debug, Serialize)]
struct MemoryReport {
    ok: bool,
    tiers: Vec<TierRow>,
    pressure: String,
    prefetch_cap: u64,
}

#[derive(Debug, Serialize)]
struct TierRow {
    tier: String,
    total: u64,
    available: u64,
    reserve: u64,
    usable: u64,
    addressable: bool,
}

/// Print the memory picture.
pub fn memory(ctx: &Context, json: bool) -> Result<bool> {
    let snapshot = collect();
    let config = &ctx.config.memory;

    let mut tiers = Vec::new();

    // L0: VRAM, per device, never summed into one pool.
    for device in &snapshot.gpus.value {
        let budget = TierBudget::compute(
            Tier::GpuVram,
            device.total_vram_bytes.unwrap_or(0),
            device.free_vram_bytes.unwrap_or(0),
            config.gpu_reserve_bytes + config.gpu_safety_margin_bytes,
        );
        tiers.push(TierRow {
            tier: format!("gpu_vram ({})", device.name),
            total: budget.total,
            available: budget.available,
            reserve: budget.reserve,
            usable: budget.usable,
            addressable: true,
        });
    }

    // L1: host RAM.
    let (total_ram, available_ram) = host_memory();
    let ram = TierBudget::compute(
        Tier::HostRam,
        total_ram,
        available_ram,
        config.ram_reserve_bytes,
    );
    let pressure = Pressure::classify(available_ram, config.ram_reserve_bytes);
    tiers.push(TierRow {
        tier: "host_ram".to_owned(),
        total: ram.total,
        available: ram.available,
        reserve: ram.reserve,
        usable: ram.usable,
        addressable: true,
    });

    // L2: NVMe, reported as a cold tier.
    for volume in snapshot.storage.value.iter().take(1) {
        let nvme = TierBudget::compute(
            Tier::Nvme,
            volume.total_bytes,
            volume.available_bytes,
            config.nvme_reserve_bytes,
        );
        tiers.push(TierRow {
            tier: format!("nvme ({})", volume.mount_point),
            total: nvme.total,
            available: nvme.available,
            reserve: nvme.reserve,
            usable: nvme.usable,
            addressable: Tier::Nvme.is_addressable_memory(),
        });
    }

    let report = MemoryReport {
        ok: true,
        tiers,
        pressure: pressure.as_str().to_owned(),
        prefetch_cap: config.max_prefetch_bytes,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(true);
    }

    println!("MEMORY TIERS");
    for row in &report.tiers {
        let kind = if row.addressable { "memory" } else { "cold" };
        println!(
            "  {:<34} {:>10} usable of {:>10} ({}, reserve {})",
            row.tier,
            output::human_bytes(row.usable),
            output::human_bytes(row.total),
            kind,
            output::human_bytes(row.reserve)
        );
    }

    println!();
    println!("pressure : {}", report.pressure);

    // Show what prefetch would be granted right now, so the policy is visible.
    match grant_prefetch(
        PrefetchRequest {
            bytes: report.prefetch_cap,
        },
        report.prefetch_cap,
        pressure,
        0,
    ) {
        Ok(granted) => println!(
            "prefetch : up to {} (cap {})",
            output::human_bytes(granted),
            output::human_bytes(report.prefetch_cap)
        ),
        Err(err) => println!("prefetch : unavailable ({})", err.0),
    }

    Ok(true)
}
