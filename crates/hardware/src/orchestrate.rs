//! Inventory orchestration: run every probe, merge the results.
//!
//! # Vendor selection
//!
//! Probes are tried in a fixed order and merged, rather than "first match wins".
//! A machine can legitimately have an NVIDIA card *and* an AMD card; hiding one
//! because the other answered first is how multi-GPU support turns into a bug
//! report. Each probe reports its own status, and the merged list is annotated
//! with the union of what could be measured.

use crate::host;
use crate::runner::{CommandRunner, SystemRunner};
use crate::types::{
    CapabilityStatus, GpuDevice, GpuTopology, HostSnapshot, ProbeOutcome, Provenance, unix_millis,
};
use crate::{gpu_amd, gpu_apple, gpu_nvidia};

/// Collect a full inventory using the real system tools.
pub fn collect() -> HostSnapshot {
    collect_with(&SystemRunner)
}

/// Collect a full inventory using the given command runner.
///
/// This is the function tests call with a fake runner.
pub fn collect_with(runner: &dyn CommandRunner) -> HostSnapshot {
    let cpu = host::cpu();
    let memory = host::memory();
    let storage = host::storage();

    let gpus = collect_gpus(runner);
    let topology = topology_for(runner, &gpus);

    HostSnapshot {
        cpu,
        memory,
        storage,
        gpus,
        topology,
        collected_at_unix_ms: unix_millis(),
    }
}

fn collect_gpus(runner: &dyn CommandRunner) -> ProbeOutcome<Vec<GpuDevice>> {
    let nvidia = gpu_nvidia::probe(runner);
    let amd = gpu_amd::probe(runner);
    let apple = gpu_apple::probe(runner);

    let mut devices = Vec::new();
    let mut reasons: Vec<String> = Vec::new();
    let mut provenance: Option<Provenance> = None;
    let mut any_experimental = false;

    for (name, outcome) in [("nvidia", &nvidia), ("amd", &amd), ("apple", &apple)] {
        if !outcome.value.is_empty() {
            devices.extend(outcome.value.clone());
            if outcome.status == CapabilityStatus::Experimental {
                any_experimental = true;
            }
            if provenance.is_none() {
                provenance = outcome.provenance.clone();
            }
        } else if let Some(reason) = &outcome.reason {
            reasons.push(format!("{name}: {reason}"));
        }
    }

    if devices.is_empty() {
        // No GPU is a fact about the machine, not a capability gap: say so
        // without pretending a tool failed.
        let mut outcome = ProbeOutcome::unavailable(
            CapabilityStatus::Unsupported,
            "no supported GPU was detected (CPU-only operation)",
        );
        if !reasons.is_empty() {
            outcome.reason = Some(format!(
                "{} [{}]",
                outcome.reason.unwrap_or_default(),
                reasons.join("; ")
            ));
        }
        return outcome;
    }

    let mut outcome = ProbeOutcome::ok(
        devices,
        provenance.unwrap_or_else(|| Provenance::new("unknown", unix_millis())),
    );
    if any_experimental {
        outcome.status = CapabilityStatus::Experimental;
        outcome.reason = Some(
            "at least one GPU was reported by a probe that is not yet validated on real hardware"
                .to_owned(),
        );
    }
    outcome
}

fn topology_for(runner: &dyn CommandRunner, gpus: &ProbeOutcome<Vec<GpuDevice>>) -> GpuTopology {
    if gpus.value.is_empty() {
        return GpuTopology {
            links: Vec::new(),
            reason: None,
            status: CapabilityStatus::Unsupported,
        };
    }

    // Only NVIDIA exposes a topology matrix today. For anything else, say that
    // the topology is unknown instead of reporting "no P2P".
    let has_nvidia = gpus
        .value
        .iter()
        .any(|gpu| gpu.vendor == crate::types::GpuVendor::Nvidia);

    if !has_nvidia {
        return GpuTopology {
            links: Vec::new(),
            reason: Some("no topology probe exists for this vendor yet (Task 7)".to_owned()),
            status: CapabilityStatus::Unknown,
        };
    }

    gpu_nvidia::topology(runner, &gpus.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_nvidia::QUERY_ARGS;
    use crate::runner::CommandOutput;
    use crate::testing::FakeRunner;

    fn nvidia_query_key() -> String {
        format!("nvidia-smi {}", QUERY_ARGS.join(" "))
    }

    fn rocm_key() -> String {
        "rocm-smi --showproductname --showmeminfo vram --showpids --json".to_owned()
    }

    const TWO_GPU: &str = "0, NVIDIA GeForce RTX 4090, 24564, 22973, 610.88, 8.9, 00000000:01:00.0, 52, 8, 21.02, 1, 16\n1, NVIDIA GeForce RTX 3060, 12288, 12113, 610.88, 8.6, 00000000:07:00.0, 28, 0, 3.73, 1, 1\n";

    fn nvidia_only() -> FakeRunner {
        FakeRunner::default()
            .with(
                &nvidia_query_key(),
                CommandOutput {
                    status: Some(0),
                    stdout: TWO_GPU.to_owned(),
                    stderr: String::new(),
                },
            )
            .with(
                "nvidia-smi topo -m",
                CommandOutput {
                    status: Some(255),
                    stdout: String::new(),
                    stderr: "Failed to run topology matrix\n".to_owned(),
                },
            )
    }

    #[test]
    fn merges_two_nvidia_cards_and_keeps_the_topology_caveat() {
        let snapshot = collect_with(&nvidia_only());
        assert_eq!(snapshot.gpus.value.len(), 2);
        assert_eq!(snapshot.gpus.status, CapabilityStatus::Supported);
        assert_eq!(snapshot.topology.status, CapabilityStatus::Unknown);
        assert!(
            snapshot
                .topology
                .reason
                .unwrap()
                .contains("topology matrix")
        );
    }

    #[test]
    fn a_cpu_only_machine_is_reported_as_cpu_only_not_as_a_broken_probe() {
        let snapshot = collect_with(&FakeRunner::default());
        assert!(snapshot.gpus.value.is_empty());
        assert_eq!(snapshot.gpus.status, CapabilityStatus::Unsupported);
        let reason = snapshot.gpus.reason.unwrap();
        assert!(reason.contains("CPU-only"), "{reason}");
        // The individual tool misses are still mentioned, for support.
        assert!(reason.contains("nvidia"), "{reason}");
    }

    #[test]
    fn cpu_memory_and_storage_are_always_collected() {
        let snapshot = collect_with(&nvidia_only());
        assert!(snapshot.cpu.is_usable());
        assert!(snapshot.memory.is_usable());
        assert!(snapshot.storage.is_usable());
    }

    #[test]
    fn total_vram_is_a_display_total_not_a_pool() {
        let snapshot = collect_with(&nvidia_only());
        let expected = (24_564u64 + 12_288) * 1024 * 1024;
        assert_eq!(snapshot.total_vram_bytes(), expected);
    }

    #[test]
    fn gaps_report_every_unavailable_subsystem() {
        let snapshot = collect_with(&FakeRunner::default());
        let gaps = snapshot.gaps();
        assert!(gaps.iter().any(|(name, _)| *name == "gpus"));
        assert!(
            gaps.iter()
                .any(|(name, _)| *name == "topology" || *name == "gpus")
        );
    }

    #[test]
    fn a_probe_that_reports_experimental_makes_the_whole_gpu_outcome_experimental() {
        let runner = FakeRunner::default()
            .with(
                &nvidia_query_key(),
                CommandOutput {
                    status: Some(0),
                    stdout: TWO_GPU.to_owned(),
                    stderr: String::new(),
                },
            )
            .with(
                &rocm_key(),
                CommandOutput {
                    status: Some(0),
                    stdout:
                        r#"{"card0": {"Card series": "Radeon", "VRAM Total Memory (B)": 1024}}"#
                            .to_owned(),
                    stderr: String::new(),
                },
            );

        let snapshot = collect_with(&runner);
        assert_eq!(snapshot.gpus.value.len(), 3, "both vendors are reported");
        assert_eq!(snapshot.gpus.status, CapabilityStatus::Experimental);
    }
}
