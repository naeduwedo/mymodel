//! Apple GPU probe (spec §2.4, §5.2).
//!
//! Apple Silicon shares one memory pool between CPU and GPU. Reporting it as
//! "24 GiB VRAM" would be wrong twice over: there is no separate pool, and the
//! number a discrete card would report does not exist here. So this probe
//! reports `total_vram_bytes: None` — the budget comes from the unified memory
//! described by [`crate::host::memory`] instead (spec §5.2, ADR-006).
//!
//! Source of truth: `system_profiler SPDisplaysDataType -json`, which is what the
//! OS itself reports to the About dialog.

use serde_json::Value;

use crate::runner::CommandRunner;
use crate::types::{
    CapabilityStatus, ComputeBackend, GpuDevice, GpuVendor, ProbeOutcome, Provenance, unix_millis,
};

/// Probe the integrated Apple GPU.
pub fn probe(runner: &dyn CommandRunner) -> ProbeOutcome<Vec<GpuDevice>> {
    if !cfg!(target_os = "macos") {
        return ProbeOutcome::unavailable(
            CapabilityStatus::Unsupported,
            "Apple GPU probing is only meaningful on macOS",
        );
    }

    let output = match runner.run("system_profiler", &["SPDisplaysDataType", "-json"]) {
        Ok(output) => output,
        Err(err) => {
            return ProbeOutcome::unavailable(
                CapabilityStatus::Unknown,
                format!("system_profiler is not available ({err})"),
            );
        }
    };

    if !output.success() {
        return ProbeOutcome::unavailable(
            CapabilityStatus::Unsupported,
            format!("system_profiler failed: {}", output.stderr_line()),
        );
    }

    match parse(&output.stdout) {
        Ok(devices) if !devices.is_empty() => {
            ProbeOutcome::ok(devices, Provenance::new("system_profiler", unix_millis()))
        }
        Ok(_) => ProbeOutcome::unavailable(
            CapabilityStatus::Unsupported,
            "system_profiler reported no GPU",
        ),
        Err(err) => ProbeOutcome::unavailable(
            CapabilityStatus::Unknown,
            format!("system_profiler output could not be parsed: {err}"),
        ),
    }
}

/// Parse `system_profiler SPDisplaysDataType -json`.
///
/// Every Apple GPU is treated as unified-memory: `total_vram_bytes` stays `None`
/// unless the machine is an Intel Mac with a discrete card, in which case the
/// reported `spdisplays_vram` is used.
pub fn parse(stdout: &str) -> Result<Vec<GpuDevice>, serde_json::Error> {
    let root: Value = serde_json::from_str(stdout)?;
    let entries = root
        .get("SPDisplaysDataType")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut devices = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let name = entry
            .get("sppci_model")
            .or_else(|| entry.get("_name"))
            .and_then(Value::as_str)
            .unwrap_or("Apple GPU")
            .trim()
            .to_owned();

        let unified = !name.to_ascii_lowercase().contains("amd")
            && !name.to_ascii_lowercase().contains("radeon");

        let total = if unified {
            None
        } else {
            entry
                .get("spdisplays_vram")
                .and_then(Value::as_str)
                .and_then(parse_vram_string)
        };

        devices.push(GpuDevice {
            id: format!("apple:{index}"),
            name,
            vendor: GpuVendor::Apple,
            backend: ComputeBackend::Metal,
            total_vram_bytes: total,
            free_vram_bytes: None,
            memory_bandwidth_bytes_sec: None,
            compute_capability: None,
            pci_bus_id: None,
            pcie_generation: None,
            pcie_lanes: None,
            numa_node: None,
            temperature_c: None,
            utilization_percent: None,
            power_watts: None,
            peer_access: Vec::new(),
            driver_version: None,
        });
    }

    Ok(devices)
}

/// `spdisplays_vram` looks like `"8 GB"` or `"1536 MB"`.
fn parse_vram_string(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let number: u64 = parts.next()?.parse().ok()?;
    let unit = parts.next()?.to_ascii_uppercase();
    let multiplier = match unit.as_str() {
        "GB" => 1024 * 1024 * 1024,
        "MB" => 1024 * 1024,
        _ => return None,
    };
    Some(number * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPLE_SILICON: &str = r#"{"SPDisplaysDataType": [{"sppci_model": "Apple M3 Max", "sppci_cores": "40", "spdisplays_metal": "supported"}]}"#;

    #[test]
    fn apple_silicon_reports_no_separate_vram_pool() {
        let devices = parse(APPLE_SILICON).unwrap();
        assert_eq!(devices.len(), 1);
        let gpu = &devices[0];
        assert_eq!(gpu.vendor, GpuVendor::Apple);
        assert_eq!(gpu.backend, ComputeBackend::Metal);
        assert_eq!(
            gpu.total_vram_bytes, None,
            "unified memory must not be reported as discrete VRAM (spec §5.2)"
        );
    }

    #[test]
    fn a_discrete_card_in_an_intel_mac_keeps_its_reported_vram() {
        let json = r#"{"SPDisplaysDataType": [{"sppci_model": "AMD Radeon Pro 5500M", "spdisplays_vram": "8 GB"}]}"#;
        let devices = parse(json).unwrap();
        assert_eq!(devices[0].total_vram_bytes, Some(8 * 1024 * 1024 * 1024));
    }

    #[test]
    fn vram_strings_are_parsed_not_guessed() {
        assert_eq!(parse_vram_string("8 GB"), Some(8 * 1024 * 1024 * 1024));
        assert_eq!(parse_vram_string("1536 MB"), Some(1536 * 1024 * 1024));
        assert_eq!(parse_vram_string("8"), None);
        assert_eq!(parse_vram_string("lots GB"), None);
    }

    #[test]
    fn probing_on_a_non_apple_platform_is_unsupported_with_a_reason() {
        let outcome = probe(&crate::runner::SystemRunner);
        if !cfg!(target_os = "macos") {
            assert_eq!(outcome.status, CapabilityStatus::Unsupported);
            assert!(outcome.reason.unwrap().contains("only meaningful on macOS"));
        }
    }
}
