//! `gguf hardware` - the inventory the Hardware page will render.
//!
//! Prints exactly what `HardwareService/GetHardware` returns, including the
//! status and reason of every probe that could not produce data. That output is
//! what a support request should contain: it says *what* was measured, *by what*,
//! and *what could not be measured and why*.

use anyhow::{Context as _, Result};
use gguf_engine_protocol::pb;

use crate::commands::Context;
use crate::output;

const STATUS_WORD: [&str; 6] = [
    "?",
    "supported",
    "unsupported",
    "experimental",
    "unknown",
    "planned",
];

fn status_word(value: i32) -> &'static str {
    STATUS_WORD.get(value as usize).copied().unwrap_or("?")
}

/// Shorten a PCI id for the topology lines ("0000:01:00.0" -> "01:00.0").
fn short_id(id: &str) -> String {
    id.rsplit_once(':')
        .map(|(_, tail)| tail.to_owned())
        .unwrap_or_else(|| id.to_owned())
}

fn bytes(value: u64) -> String {
    output::human_bytes(value)
}

/// Fetch and print the hardware report. Returns false when no core is running.
pub async fn hardware(ctx: &Context, json: bool) -> Result<bool> {
    let client = match gguf_engine_ipc::connect(ctx.paths.root()).await {
        Ok(client) => client,
        Err(gguf_engine_ipc::IpcError::NotRunning(endpoint)) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "ok": false, "reason": "core is not running", "endpoint": endpoint })
                );
            } else {
                println!("core is not running (endpoint {endpoint})");
            }
            return Ok(false);
        }
        Err(err) => return Err(err).context("connecting to the core"),
    };

    let report = client
        .hardware()
        .await
        .context("fetching the hardware report")?;

    if json {
        println!("{}", serde_json::to_string_pretty(&view(&report))?);
        return Ok(true);
    }

    if let Some(cpu) = &report.cpu {
        println!("CPU");
        println!("  model           : {}", cpu.model);
        println!(
            "  cores           : {} physical, {} logical",
            cpu.physical_cores, cpu.logical_cores
        );
        println!("  base clock      : {} MHz", cpu.base_frequency_mhz);
        println!("  architecture    : {}", cpu.architecture);
    }

    if let Some(memory) = &report.memory {
        println!();
        println!("MEMORY");
        println!(
            "  total           : {} ({} available)",
            bytes(memory.total_bytes),
            bytes(memory.available_bytes)
        );
        if memory.unified_with_gpu {
            println!("  topology        : unified with the GPU (no separate VRAM pool)");
        }
    }

    println!();
    println!("GPUS   status: {}", status_word(report.status));
    if !report.unavailable_reason.is_empty() {
        println!("  reason          : {}", report.unavailable_reason);
    }
    for gpu in &report.gpus {
        let vram = gpu
            .total_vram_bytes
            .map(|total| {
                let free = gpu.free_vram_bytes.unwrap_or(0);
                format!("{} total, {} free", bytes(total), bytes(free))
            })
            .unwrap_or_else(|| "no discrete VRAM (unified memory)".to_owned());
        println!("  [{}] {}", gpu.vendor, gpu.name);
        println!("        backend   : {}", gpu.backend);
        println!("        vram      : {vram}");
        if let Some(id) = gpu.pci_bus_id.as_deref() {
            let link = match (gpu.pcie_generation, gpu.pcie_lanes) {
                (Some(generation), Some(lanes)) => {
                    format!(" (PCIe gen{generation} x{lanes})")
                }
                _ => String::new(),
            };
            println!("        pci       : {id}{link} (link as currently negotiated)");
        }
        if let Some(cap) = gpu.compute_capability.as_deref() {
            println!("        compute   : {cap}");
        }
        if !gpu.driver_version.is_empty() {
            println!("        driver    : {}", gpu.driver_version);
        }
        let mut live = Vec::new();
        if let Some(t) = gpu.temperature_c {
            live.push(format!("{t:.0} C"));
        }
        if let Some(u) = gpu.utilization_percent {
            live.push(format!("{u:.0}% util"));
        }
        if let Some(p) = gpu.power_watts {
            live.push(format!("{p:.0} W"));
        }
        if !live.is_empty() {
            println!("        now       : {}", live.join(", "));
        }
    }

    if let Some(topology) = &report.topology {
        println!();
        let lines: Vec<String> = topology
            .links
            .iter()
            .map(|link| {
                let from = short_id(&link.from_device);
                let to = short_id(&link.to_device);
                let p2p = if link.p2p_supported_by_driver == pb::CapabilityStatus::Supported as i32
                {
                    "p2p capable"
                } else {
                    "host-mediated"
                };
                format!("{from} <-> {to}: {} ({p2p})", link.interconnect)
            })
            .collect();
        println!("TOPOLOGY   status: {}", status_word(topology.status));
        if !topology.unavailable_reason.is_empty() {
            println!("  unavailable     : {}", topology.unavailable_reason);
        }
        for line in lines {
            println!("  {line}");
        }
        if topology
            .links
            .iter()
            .all(|l| l.p2p_tested != pb::CapabilityStatus::Supported as i32)
        {
            println!(
                "  note            : P2P capability is detected, not measured; bandwidth is Task 7"
            );
        }
    }

    println!();
    println!("STORAGE");
    for volume in &report.storage {
        println!(
            "  {:<12} {:>10} free of {:>10}{}",
            volume.mount_point,
            bytes(volume.available_bytes),
            bytes(volume.total_bytes),
            if volume.removable { " (removable)" } else { "" }
        );
    }

    Ok(true)
}

fn view(report: &pb::HardwareReport) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "collected_at_unix_ms": report.collected_at_unix_ms,
        "cpu": report.cpu.as_ref().map(|cpu| serde_json::json!({
            "model": cpu.model,
            "physical_cores": cpu.physical_cores,
            "logical_cores": cpu.logical_cores,
            "base_frequency_mhz": cpu.base_frequency_mhz,
            "architecture": cpu.architecture,
        })),
        "memory": report.memory.as_ref().map(|memory| serde_json::json!({
            "total_bytes": memory.total_bytes,
            "available_bytes": memory.available_bytes,
            "unified_with_gpu": memory.unified_with_gpu,
        })),
        "gpus": {
            "status": status_word(report.status),
            "reason": report.unavailable_reason,
            "devices": report.gpus.iter().map(|gpu| serde_json::json!({
                "id": gpu.id,
                "name": gpu.name,
                "vendor": gpu.vendor,
                "backend": gpu.backend,
                "total_vram_bytes": gpu.total_vram_bytes,
                "free_vram_bytes": gpu.free_vram_bytes,
                "compute_capability": gpu.compute_capability,
                "pci_bus_id": gpu.pci_bus_id,
                "pcie_generation": gpu.pcie_generation,
                "pcie_lanes": gpu.pcie_lanes,
                "temperature_c": gpu.temperature_c,
                "utilization_percent": gpu.utilization_percent,
                "power_watts": gpu.power_watts,
                "driver_version": gpu.driver_version,
            })).collect::<Vec<_>>(),
        },
        "topology": report.topology.as_ref().map(|topology| serde_json::json!({
            "status": status_word(topology.status),
            "reason": topology.unavailable_reason,
            "links": topology.links.iter().map(|link| serde_json::json!({
                "from": link.from_device,
                "to": link.to_device,
                "interconnect": link.interconnect,
                "p2p_detected": status_word(link.p2p_detected),
                "p2p_supported_by_driver": status_word(link.p2p_supported_by_driver),
                "p2p_tested": status_word(link.p2p_tested),
            })).collect::<Vec<_>>(),
        })),
        "storage": report.storage.iter().map(|volume| serde_json::json!({
            "mount_point": volume.mount_point,
            "total_bytes": volume.total_bytes,
            "available_bytes": volume.available_bytes,
            "removable": volume.removable,
        })).collect::<Vec<_>>(),
    })
}
