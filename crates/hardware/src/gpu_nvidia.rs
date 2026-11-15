//! NVIDIA probe (spec §4.2, §5.2, §5.3).
//!
//! # Why `nvidia-smi` and not NVML bindings
//!
//! NVML would be faster, but calling it means `unsafe` FFI to `nvml.dll` and a
//! driver-version matrix. `nvidia-smi` is shipped by the same driver, reports the
//! same numbers, and lets this crate stay free of `unsafe`. The trade-off is
//! documented here rather than hidden: every value carries provenance
//! (`nvidia-smi <version>`), so a reader always knows what produced it.
//!
//! # What this probe refuses to do
//!
//! * It never derives VRAM from a device name.
//! * It never reports P2P as usable because a matrix exists: the matrix says
//!   *detected*, and `p2p_tested` stays `Unsupported` until Task 7 measures it.
//! * When `nvidia-smi topo -m` fails — which happens on some Windows driver
//!   builds — topology is reported as `Unknown` **with the driver error text**,
//!   not as "no P2P".

use crate::runner::CommandRunner;
use crate::types::{
    CapabilityStatus, ComputeBackend, GpuDevice, GpuTopology, GpuVendor, Interconnect, PeerLink,
    ProbeOutcome, Provenance, unix_millis,
};

/// Query fields, in the order the parser expects them.
const QUERY_FIELDS: &str = "index,name,memory.total,memory.free,driver_version,compute_cap,pci.bus_id,temperature.gpu,utilization.gpu,power.draw,pcie.link.gen.current,pcie.link.width.current";

/// Arguments for the inventory query.
///
/// Exposed to the crate so tests build their fake-runner keys from the same
/// values the probe uses; a duplicated literal in a test is a test that stops
/// covering the call the moment the call changes.
pub(crate) const QUERY_ARGS: [&str; 3] =
    ["--query-gpu", QUERY_FIELDS, "--format=csv,noheader,nounits"];

/// Probe every NVIDIA GPU.
pub fn probe(runner: &dyn CommandRunner) -> ProbeOutcome<Vec<GpuDevice>> {
    let output = match runner.run("nvidia-smi", &QUERY_ARGS) {
        Ok(output) => output,
        Err(err) => {
            // No tool at all: this is "we could not ask", not "no GPU".
            return ProbeOutcome::unavailable(
                CapabilityStatus::Unknown,
                format!("nvidia-smi is not available ({err})"),
            );
        }
    };

    if !output.success() {
        return ProbeOutcome::unavailable(
            CapabilityStatus::Unsupported,
            format!("nvidia-smi failed: {}", output.stderr_line()),
        );
    }

    let devices = parse_query(&output.stdout);
    if devices.is_empty() {
        return ProbeOutcome::unavailable(
            CapabilityStatus::Unknown,
            "nvidia-smi returned no GPU rows",
        );
    }

    let version = driver_version(runner);
    let mut provenance = Provenance::new("nvidia-smi", unix_millis());
    if let Some(version) = version.clone() {
        provenance = provenance.with_version(version);
    }

    ProbeOutcome::ok(devices, provenance)
}

/// Driver version, best effort (`nvidia-smi --version` output).
fn driver_version(runner: &dyn CommandRunner) -> Option<String> {
    let output = runner.run("nvidia-smi", &["--version"]).ok()?;
    if !output.success() {
        return None;
    }

    output
        .stdout
        .lines()
        .find_map(|line| {
            line.split_once("KMD version")
                .map(|(_, v)| v.trim().to_owned())
        })
        .or_else(|| {
            output.stdout.lines().find_map(|line| {
                line.split_once("Driver Version")
                    .map(|(_, v)| v.trim().to_owned())
            })
        })
}

/// Parse the CSV rows of `--query-gpu`.
///
/// Rows are parsed field by field with explicit `None` for `[N/A]`, so a driver
/// that stops reporting, say, power draw cannot silently shift every later field.
pub fn parse_query(stdout: &str) -> Vec<GpuDevice> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(parse_row)
        .collect()
}

fn parse_row(line: &str) -> Option<GpuDevice> {
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();
    if fields.len() < 10 {
        return None;
    }

    let index = fields[0];
    let name = fields[1].to_owned();
    let pci_bus_id = parse_text(fields[6]);

    Some(GpuDevice {
        id: pci_bus_id
            .clone()
            .unwrap_or_else(|| format!("nvidia:{index}")),
        vendor: GpuVendor::Nvidia,
        backend: ComputeBackend::Cuda,
        total_vram_bytes: parse_mebibytes(fields[2]),
        free_vram_bytes: parse_mebibytes(fields[3]),
        memory_bandwidth_bytes_sec: None,
        compute_capability: parse_text(fields[5]),
        pci_bus_id,
        pcie_generation: fields.get(10).and_then(|v| parse_number::<u32>(v)),
        pcie_lanes: fields.get(11).and_then(|v| parse_number::<u32>(v)),
        numa_node: None,
        temperature_c: parse_number::<f32>(fields[7]),
        utilization_percent: parse_number::<f32>(fields[8]),
        power_watts: parse_number::<f32>(fields[9]),
        peer_access: Vec::new(),
        driver_version: parse_text(fields[4]),
        name,
    })
}

fn parse_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "[N/A]" || trimmed == "N/A" {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn parse_number<T: std::str::FromStr>(value: &str) -> Option<T> {
    parse_text(value).and_then(|text| text.parse::<T>().ok())
}

/// `nvidia-smi` reports MiB even with `nounits`.
fn parse_mebibytes(value: &str) -> Option<u64> {
    let mib: u64 = parse_number(value)?;
    Some(mib * 1024 * 1024)
}

/// Probe the topology matrix, reporting honestly when the driver refuses.
pub fn topology(runner: &dyn CommandRunner, devices: &[GpuDevice]) -> GpuTopology {
    if devices.len() < 2 {
        return GpuTopology {
            links: Vec::new(),
            reason: None,
            status: CapabilityStatus::Supported,
        };
    }

    let output = match runner.run("nvidia-smi", &["topo", "-m"]) {
        Ok(output) => output,
        Err(err) => {
            return GpuTopology {
                links: Vec::new(),
                reason: Some(format!("nvidia-smi is not available ({err})")),
                status: CapabilityStatus::Unknown,
            };
        }
    };

    if !output.success() {
        // Observed on real Windows drivers: the header is printed, the matrix is
        // not, and the tool exits non-zero. Reporting "no P2P" here would be a
        // fabrication (spec §5.3).
        return GpuTopology {
            links: Vec::new(),
            reason: Some(format!(
                "nvidia-smi topo -m failed: {}",
                output.stderr_line()
            )),
            status: CapabilityStatus::Unknown,
        };
    }

    match parse_matrix(&output.stdout, devices) {
        Ok(links) => GpuTopology {
            links,
            reason: None,
            status: CapabilityStatus::Supported,
        },
        Err(err) => GpuTopology {
            links: Vec::new(),
            reason: Some(err),
            status: CapabilityStatus::Unknown,
        },
    }
}

/// Parse the body of `nvidia-smi topo -m`.
///
/// The format is a triangular matrix; the code is the interconnect for the pair
/// (`NV#` NVLink, `SYS` across NUMA, `PIX`/`PXB`/`PHB` PCIe paths, `NODE` same
/// NUMA node). `X` marks the diagonal.
pub fn parse_matrix(stdout: &str, devices: &[GpuDevice]) -> Result<Vec<PeerLink>, String> {
    let mut links = Vec::new();
    let mut seen_any = false;

    for line in stdout.lines().skip(1) {
        let trimmed = line.trim();
        if !trimmed.starts_with("GPU") && !trimmed.starts_with("\tGPU") {
            continue;
        }

        let columns: Vec<&str> = trimmed.split_whitespace().collect();
        if columns.len() < 3 {
            continue;
        }

        let from_index: usize = match columns[0].trim_start_matches("GPU").parse() {
            Ok(index) => index,
            Err(_) => continue,
        };

        for (offset, code) in columns[1..].iter().enumerate() {
            // The matrix is triangular: only read the upper half.
            if offset < from_index {
                continue;
            }
            if offset == from_index {
                continue;
            }
            if *code == "X" || !is_interconnect_code(code) {
                continue;
            }

            let Some(from) = devices.get(from_index) else {
                continue;
            };
            let Some(to) = devices.get(offset) else {
                continue;
            };

            seen_any = true;
            let interconnect = classify(code);
            links.push(PeerLink {
                from_device: from.id.clone(),
                to_device: to.id.clone(),
                interconnect,
                p2p_detected: interconnect_status(interconnect),
                p2p_supported_by_driver: interconnect_status(interconnect),
                // Never claimed from a matrix alone (spec §5.3).
                p2p_tested: CapabilityStatus::Unsupported,
                measured_bandwidth_bytes_sec: None,
            });
        }
    }

    if seen_any || devices.len() < 2 {
        Ok(links)
    } else {
        Err("topology matrix was printed but contained no GPU rows".to_owned())
    }
}

fn is_interconnect_code(code: &str) -> bool {
    code == "SYS"
        || code == "NODE"
        || code == "PIX"
        || code == "PXB"
        || code == "PHB"
        || code.starts_with("NV")
}

fn classify(code: &str) -> Interconnect {
    if code.starts_with("NV") {
        Interconnect::NvLink
    } else if code == "SYS" {
        Interconnect::PcieAcrossNuma
    } else if code == "PIX" || code == "PXB" || code == "PHB" || code == "NODE" {
        Interconnect::Pcie
    } else {
        Interconnect::Unknown
    }
}

fn interconnect_status(interconnect: Interconnect) -> CapabilityStatus {
    if interconnect.is_direct() {
        CapabilityStatus::Supported
    } else {
        CapabilityStatus::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::CommandOutput;
    use crate::testing::FakeRunner;

    /// The exact key the fake runner records for the inventory query.
    fn query_key() -> String {
        format!("nvidia-smi {}", QUERY_ARGS.join(" "))
    }

    // Recorded from an RTX 4090 + RTX 3060 machine (nvidia-smi 610.88).
    /// Recorded from this project's test machine: RTX 4090 + RTX 3060,
    /// driver 610.88. Kept verbatim so the parser is tested against real output
    /// rather than against a shape someone imagined.
    const REAL_QUERY: &str = concat!(
        "0, NVIDIA GeForce RTX 4090, 24564, 22973, 610.88, 8.9, 00000000:01:00.0, 52, 8, 21.02, 1, 16\n",
        "1, NVIDIA GeForce RTX 3060, 12288, 12113, 610.88, 8.6, 00000000:07:00.0, 28, 0, 3.73, 1, 1\n",
    );

    #[test]
    fn parses_the_recorded_two_gpu_machine() {
        let devices = parse_query(REAL_QUERY);
        assert_eq!(devices.len(), 2);

        let first = &devices[0];
        assert_eq!(first.name, "NVIDIA GeForce RTX 4090");
        assert_eq!(first.vendor, GpuVendor::Nvidia);
        assert_eq!(first.backend, ComputeBackend::Cuda);
        assert_eq!(first.total_vram_bytes, Some(24_564 * 1024 * 1024));
        assert_eq!(first.free_vram_bytes, Some(22_973 * 1024 * 1024));
        assert_eq!(first.compute_capability.as_deref(), Some("8.9"));
        assert_eq!(first.pci_bus_id.as_deref(), Some("00000000:01:00.0"));
        assert_eq!(first.temperature_c, Some(52.0));
        assert_eq!(first.utilization_percent, Some(8.0));
        assert_eq!(first.power_watts, Some(21.02));
        assert_eq!(first.pcie_generation, Some(1));
        assert_eq!(first.pcie_lanes, Some(16));

        let second = &devices[1];
        assert_eq!(second.total_vram_bytes, Some(12_288 * 1024 * 1024));
        assert_eq!(second.compute_capability.as_deref(), Some("8.6"));
    }

    #[test]
    fn ids_are_the_pci_bus_ids_so_they_are_stable_across_probe_runs() {
        let devices = parse_query(REAL_QUERY);
        assert_eq!(devices[0].id, "00000000:01:00.0");
        assert_ne!(devices[0].id, devices[1].id);
    }

    #[test]
    fn not_available_fields_become_none_not_zero() {
        let line = "0, Card, 100, [N/A], [N/A], 8.0, 0000:01:00.0, [N/A], [N/A], [N/A]";
        let devices = parse_query(line);
        assert_eq!(devices.len(), 1);
        let device = &devices[0];
        assert_eq!(device.free_vram_bytes, None);
        assert_eq!(device.temperature_c, None);
        assert_eq!(device.utilization_percent, None);
        assert_eq!(device.power_watts, None);
        assert_eq!(device.driver_version, None);
        // ... while the fields that *were* reported keep their values.
        assert_eq!(device.total_vram_bytes, Some(100 * 1024 * 1024));
        assert_eq!(device.compute_capability.as_deref(), Some("8.0"));
    }

    #[test]
    fn short_or_malformed_rows_are_skipped_instead_of_guessed() {
        assert!(parse_query("0, OnlyName, 100").is_empty());
        assert!(parse_query("").is_empty());
        assert!(parse_query("garbage").is_empty());
    }

    #[test]
    fn missing_tool_is_unknown_not_unsupported() {
        let runner = FakeRunner::default();
        let outcome = probe(&runner);
        assert_eq!(outcome.status, CapabilityStatus::Unknown);
        assert!(
            outcome
                .reason
                .unwrap()
                .contains("nvidia-smi is not available")
        );
    }

    #[test]
    fn a_failing_tool_reports_its_error_text() {
        let runner = FakeRunner::default().with(
            &query_key(),
            CommandOutput {
                status: Some(9),
                stdout: String::new(),
                stderr: "No devices were found\n".to_owned(),
            },
        );
        let outcome = probe(&runner);
        assert_eq!(outcome.status, CapabilityStatus::Unsupported);
        assert!(outcome.reason.unwrap().contains("No devices were found"));
    }

    #[test]
    fn modern_topology_matrix_yields_verified_pairs_only() {
        let devices = parse_query(REAL_QUERY);
        let matrix = "\tGPU0\tGPU1\tCPU Affinity\tNUMA Affinity\nGPU0\t X \tSYS\t0-31\t0\nGPU1\tSYS\t X \t0-31\t0\n";
        let links = parse_matrix(matrix, &devices).unwrap();
        assert_eq!(links.len(), 1, "each unordered pair once");
        let link = &links[0];
        assert_eq!(link.from_device, devices[0].id);
        assert_eq!(link.to_device, devices[1].id);
        assert_eq!(link.interconnect, Interconnect::PcieAcrossNuma);
        assert_eq!(link.p2p_supported_by_driver, CapabilityStatus::Unsupported);
    }

    #[test]
    fn nvlink_is_detected_but_never_reported_as_tested() {
        let devices = parse_query(REAL_QUERY);
        let matrix = "\tGPU0\tGPU1\nGPU0\t X \tNV4\nGPU1\tNV4\t X \n";
        let links = parse_matrix(matrix, &devices).unwrap();
        assert_eq!(links[0].interconnect, Interconnect::NvLink);
        assert_eq!(links[0].p2p_detected, CapabilityStatus::Supported);
        assert_eq!(
            links[0].p2p_tested,
            CapabilityStatus::Unsupported,
            "detection is not a measurement (spec §5.3)"
        );
        assert!(links[0].measured_bandwidth_bytes_sec.is_none());
    }

    #[test]
    fn a_failed_matrix_is_unknown_with_the_driver_error() {
        let devices = parse_query(REAL_QUERY);
        let runner = FakeRunner::default()
            .with(
                &query_key(),
                CommandOutput {
                    status: Some(0),
                    stdout: REAL_QUERY.to_owned(),
                    stderr: String::new(),
                },
            )
            .with(
                "nvidia-smi topo -m",
                CommandOutput {
                    status: Some(255),
                    stdout: "\tGPU0\tGPU1\n".to_owned(),
                    stderr: "Failed to run topology matrix\n".to_owned(),
                },
            );

        let topology = topology(&runner, &devices);
        assert_eq!(topology.status, CapabilityStatus::Unknown);
        assert!(topology.links.is_empty());
        assert!(
            topology
                .reason
                .unwrap()
                .contains("Failed to run topology matrix"),
            "the user must see the driver's own words"
        );
    }

    #[test]
    fn a_single_gpu_needs_no_topology() {
        let devices = vec![GpuDevice {
            id: "0".into(),
            name: "Card".into(),
            vendor: GpuVendor::Nvidia,
            backend: ComputeBackend::Cuda,
            total_vram_bytes: Some(1),
            free_vram_bytes: Some(1),
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
        }];
        let runner = FakeRunner::default();
        let topology = topology(&runner, &devices);
        assert_eq!(topology.status, CapabilityStatus::Supported);
        assert!(topology.reason.is_none());
        assert!(topology.links.is_empty());
    }
}
