//! AMD probe (spec §4.2, §5.2).
//!
//! # Status: experimental, and honest about it
//!
//! `rocm-smi --json` is the documented way to read AMD VRAM without linking
//! ROCm. It is implemented here against the documented JSON shape and covered by
//! parsing tests, but it has **not been run against a real AMD card** by the team
//! that wrote it. The probe therefore reports [`CapabilityStatus::Experimental`]
//! instead of `Supported`, so the UI can say "detected, not yet validated"
//! rather than claiming a measurement it cannot defend (spec §14.2: mock tests
//! must never be reported as hardware tests).

use serde::Deserialize;

use crate::runner::CommandRunner;
use crate::types::{
    CapabilityStatus, ComputeBackend, GpuDevice, GpuVendor, ProbeOutcome, Provenance, unix_millis,
};

/// `rocm-smi --showmeminfo vram --showproductname --json` output.
#[derive(Debug, Deserialize)]
struct RocmReport {
    #[serde(flatten)]
    cards: std::collections::BTreeMap<String, serde_json::Value>,
}

/// Probe every AMD GPU visible to ROCm.
pub fn probe(runner: &dyn CommandRunner) -> ProbeOutcome<Vec<GpuDevice>> {
    let args = [
        "--showproductname",
        "--showmeminfo",
        "vram",
        "--showpids",
        "--json",
    ];

    let output = match runner.run("rocm-smi", &args) {
        Ok(output) => output,
        Err(err) => {
            return ProbeOutcome::unavailable(
                CapabilityStatus::Unknown,
                format!("rocm-smi is not available ({err})"),
            );
        }
    };

    if !output.success() {
        return ProbeOutcome::unavailable(
            CapabilityStatus::Unsupported,
            format!("rocm-smi failed: {}", output.stderr_line()),
        );
    }

    let devices = match parse_json(&output.stdout) {
        Ok(devices) if !devices.is_empty() => devices,
        Ok(_) => {
            return ProbeOutcome::unavailable(
                CapabilityStatus::Unknown,
                "rocm-smi reported no cards",
            );
        }
        Err(err) => {
            return ProbeOutcome::unavailable(
                CapabilityStatus::Unknown,
                format!("rocm-smi output could not be parsed: {err}"),
            );
        }
    };

    // Parsed and plausible, but never validated against real hardware:
    // `Experimental` is the honest status here.
    let mut outcome = ProbeOutcome::ok(
        devices,
        Provenance::new("rocm-smi", unix_millis()).with_version("unverified"),
    );
    outcome.status = CapabilityStatus::Experimental;
    outcome.reason = Some(
        "rocm-smi parsing is implemented but has not been validated on real AMD hardware"
            .to_owned(),
    );
    outcome
}

/// Parse the documented `rocm-smi --json` shape.
///
/// Keys are card indices ("card0", "card1"); each card carries a `VRAM Total
/// Memory (B)` and a `Card series`/`Card SKU` string. Field names vary between
/// ROCm releases, so each lookup accepts the known spellings.
pub fn parse_json(stdout: &str) -> Result<Vec<GpuDevice>, serde_json::Error> {
    let report: RocmReport = serde_json::from_str(stdout)?;
    let mut devices = Vec::new();

    for (card, value) in report.cards {
        if !card.starts_with("card") {
            continue;
        }

        let name = lookup_string(&value, &["Card series", "Card SKU", "Card name"])
            .unwrap_or_else(|| card.clone());
        let total = lookup_u64(&value, &["VRAM Total Memory (B)", "VRAM Total Memory"]);
        let used = lookup_u64(
            &value,
            &["VRAM Total Used Memory (B)", "VRAM Total Used Memory"],
        );

        devices.push(GpuDevice {
            id: card.clone(),
            name,
            vendor: GpuVendor::Amd,
            backend: ComputeBackend::Rocm,
            total_vram_bytes: total,
            free_vram_bytes: match (total, used) {
                (Some(total), Some(used)) => Some(total.saturating_sub(used)),
                _ => None,
            },
            memory_bandwidth_bytes_sec: None,
            compute_capability: None,
            pci_bus_id: None,
            pcie_generation: None,
            pcie_lanes: None,
            numa_node: None,
            temperature_c: lookup_f32(
                &value,
                &["Temperature (Sensor edge) (C)", "Temperature (C)"],
            ),
            utilization_percent: None,
            power_watts: lookup_f32(&value, &["Average Graphics Package Power (W)", "Power (W)"]),
            peer_access: Vec::new(),
            driver_version: None,
        });
    }

    devices.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(devices)
}

fn lookup_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(|v| v.as_str()) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

fn lookup_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(raw) = value.get(*key) {
            if let Some(number) = raw.as_u64() {
                return Some(number);
            }
            // ROCm sometimes reports numbers as strings.
            if let Some(parsed) = raw
                .as_str()
                .and_then(|text| text.trim().parse::<u64>().ok())
            {
                return Some(parsed);
            }
        }
    }
    None
}

fn lookup_f32(value: &serde_json::Value, keys: &[&str]) -> Option<f32> {
    for key in keys {
        if let Some(raw) = value.get(*key) {
            if let Some(number) = raw.as_f64() {
                return Some(number as f32);
            }
            if let Some(parsed) = raw
                .as_str()
                .and_then(|text| text.trim().parse::<f32>().ok())
            {
                return Some(parsed);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::CommandOutput;
    use crate::testing::FakeRunner;

    // Shape taken from the documented rocm-smi --json output. This is a schema
    // fixture, not a recording from real hardware - which is exactly why the
    // probe reports Experimental.
    const SCHEMA_FIXTURE: &str = r#"{
        "card0": {
            "Card series": "Radeon AI PRO R9700",
            "VRAM Total Memory (B)": "34359738368",
            "VRAM Total Used Memory (B)": "1073741824",
            "Temperature (Sensor edge) (C)": "41.0",
            "Average Graphics Package Power (W)": "37.0"
        },
        "system": { "Driver version": "6.10.5" }
    }"#;

    #[test]
    fn parses_the_documented_schema() {
        let devices = parse_json(SCHEMA_FIXTURE).unwrap();
        assert_eq!(devices.len(), 1, "the \"system\" key is not a card");

        let card = &devices[0];
        assert_eq!(card.id, "card0");
        assert_eq!(card.name, "Radeon AI PRO R9700");
        assert_eq!(card.vendor, GpuVendor::Amd);
        assert_eq!(card.backend, ComputeBackend::Rocm);
        assert_eq!(card.total_vram_bytes, Some(34_359_738_368));
        assert_eq!(card.free_vram_bytes, Some(34_359_738_368 - 1_073_741_824));
        assert_eq!(card.temperature_c, Some(41.0));
        assert_eq!(card.power_watts, Some(37.0));
    }

    #[test]
    fn cards_are_sorted_so_two_probes_agree() {
        let json = r#"{"card1": {"Card series": "B", "VRAM Total Memory (B)": 1},
                      "card0": {"Card series": "A", "VRAM Total Memory (B)": 1}}"#;
        let devices = parse_json(json).unwrap();
        assert_eq!(devices[0].id, "card0");
        assert_eq!(devices[1].id, "card1");
    }

    #[test]
    fn missing_used_memory_leaves_free_unknown() {
        let json = r#"{"card0": {"Card series": "X", "VRAM Total Memory (B)": 100}}"#;
        let devices = parse_json(json).unwrap();
        assert_eq!(devices[0].free_vram_bytes, None);
        assert_eq!(devices[0].total_vram_bytes, Some(100));
    }

    #[test]
    fn a_successful_probe_is_experimental_not_supported() {
        let runner = FakeRunner::default().with(
            "rocm-smi --showproductname --showmeminfo vram --showpids --json",
            CommandOutput {
                status: Some(0),
                stdout: SCHEMA_FIXTURE.to_owned(),
                stderr: String::new(),
            },
        );
        let outcome = probe(&runner);
        assert_eq!(
            outcome.status,
            CapabilityStatus::Experimental,
            "unvalidated parsing must not be advertised as supported"
        );
        assert!(outcome.is_usable(), "experimental data is still usable");
        assert!(outcome.reason.unwrap().contains("not been validated"));
    }

    #[test]
    fn missing_tool_is_unknown() {
        let outcome = probe(&FakeRunner::default());
        assert_eq!(outcome.status, CapabilityStatus::Unknown);
    }

    #[test]
    fn garbage_output_is_reported_not_guessed() {
        let runner = FakeRunner::default().with(
            "rocm-smi --showproductname --showmeminfo vram --showpids --json",
            CommandOutput {
                status: Some(0),
                stdout: "not json".to_owned(),
                stderr: String::new(),
            },
        );
        let outcome = probe(&runner);
        assert_eq!(outcome.status, CapabilityStatus::Unknown);
        assert!(outcome.reason.unwrap().contains("could not be parsed"));
    }
}
