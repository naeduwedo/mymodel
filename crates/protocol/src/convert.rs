//! Conversions between domain types and the generated Protobuf types.
//!
//! Keeping conversions in one module means the wire format can change without
//! touching scheduling or storage code, and it gives the tests a single place
//! to assert the mapping is lossless.

use gguf_engine_event_bus::{
    BusStats as DomainBusStats, CoreState as DomainState, Event as DomainEvent, EventPayload,
    NoticeLevel,
};

use crate::pb;

/// Core lifecycle state → wire enum.
pub fn core_state(state: DomainState) -> pb::CoreState {
    match state {
        DomainState::Stopped => pb::CoreState::Stopped,
        DomainState::Starting => pb::CoreState::Starting,
        DomainState::Ready => pb::CoreState::Ready,
        DomainState::Degraded => pb::CoreState::Degraded,
        DomainState::ShuttingDown => pb::CoreState::ShuttingDown,
    }
}

/// Wire enum → core lifecycle state (unknown values become `Stopped`).
pub fn core_state_from_pb(value: i32) -> DomainState {
    match pb::CoreState::try_from(value) {
        Ok(pb::CoreState::Starting) => DomainState::Starting,
        Ok(pb::CoreState::Ready) => DomainState::Ready,
        Ok(pb::CoreState::Degraded) => DomainState::Degraded,
        Ok(pb::CoreState::ShuttingDown) => DomainState::ShuttingDown,
        _ => DomainState::Stopped,
    }
}

/// Event bus statistics → wire message.
pub fn bus_stats(stats: DomainBusStats) -> pb::BusStats {
    pb::BusStats {
        published: stats.published,
        dropped: stats.dropped,
        subscribers: u32::try_from(stats.subscribers).unwrap_or(u32::MAX),
        capacity: u32::try_from(stats.capacity).unwrap_or(u32::MAX),
        last_seq: stats.last_seq,
    }
}

/// Build a wire component from plain parts.
///
/// `engine-core` converts its own `ComponentHealth` through this function so
/// the protocol crate never depends on the core crate.
pub fn component_parts(name: &str, healthy: bool, detail: Option<&str>) -> pb::ComponentHealth {
    pb::ComponentHealth {
        name: name.to_owned(),
        healthy,
        detail: detail.unwrap_or_default().to_owned(),
    }
}

/// A domain event → wire message.
///
/// The payload is carried as JSON: the wire keeps a stable envelope (seq,
/// timestamp, correlation id) while the payload schema can evolve inside the
/// protocol version. Clients that need strict typing parse the JSON into
/// `EventPayload`.
pub fn event_message(event: &DomainEvent) -> pb::Event {
    pb::Event {
        seq: event.seq,
        at_unix_ms: event.at_unix_ms,
        correlation_id: event.correlation_id.clone().unwrap_or_default(),
        payload_json: serde_json::to_string(&event.payload).unwrap_or_else(|err| {
            format!(
                "{{\"type\":\"serialisation_error\",\"data\":{{\"message\":{}}}}}",
                serde_json::Value::String(err.to_string())
            )
        }),
    }
}

/// Parse a wire event payload back into a domain payload.
pub fn payload_from_json(json: &str) -> Option<EventPayload> {
    serde_json::from_str(json).ok()
}

/// Notice severity name, used by structured logging.
pub fn notice_level_name(level: NoticeLevel) -> &'static str {
    match level {
        NoticeLevel::Info => "info",
        NoticeLevel::Warning => "warning",
        NoticeLevel::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_states_round_trip() {
        for state in [
            DomainState::Stopped,
            DomainState::Starting,
            DomainState::Ready,
            DomainState::Degraded,
            DomainState::ShuttingDown,
        ] {
            let wire = core_state(state);
            assert_eq!(core_state_from_pb(wire as i32), state);
        }
    }

    #[test]
    fn unknown_wire_state_is_treated_as_stopped() {
        assert_eq!(core_state_from_pb(0), DomainState::Stopped);
        assert_eq!(core_state_from_pb(999), DomainState::Stopped);
    }

    #[test]
    fn bus_stats_map_every_field() {
        let stats = DomainBusStats {
            published: 7,
            dropped: 2,
            subscribers: 3,
            capacity: 1024,
            last_seq: 9,
        };
        let wire = bus_stats(stats);
        assert_eq!(wire.published, 7);
        assert_eq!(wire.dropped, 2);
        assert_eq!(wire.subscribers, 3);
        assert_eq!(wire.capacity, 1024);
        assert_eq!(wire.last_seq, 9);
    }

    #[test]
    fn events_carry_a_parseable_payload() {
        let event = DomainEvent {
            seq: 4,
            at_unix_ms: 1_700_000_000_000,
            correlation_id: Some("task-1".into()),
            payload: EventPayload::CoreStateChanged {
                from: DomainState::Starting,
                to: DomainState::Ready,
            },
        };
        let wire = event_message(&event);
        assert_eq!(wire.seq, 4);
        assert_eq!(wire.correlation_id, "task-1");
        assert_eq!(payload_from_json(&wire.payload_json), Some(event.payload));
    }

    #[test]
    fn missing_correlation_id_becomes_empty_string() {
        let event = DomainEvent {
            seq: 1,
            at_unix_ms: 1,
            correlation_id: None,
            payload: EventPayload::Notice {
                level: NoticeLevel::Info,
                message: "hi".into(),
            },
        };
        assert_eq!(event_message(&event).correlation_id, "");
    }

    #[test]
    fn component_parts_keep_the_reason() {
        let wire = component_parts("ipc", false, Some("listener failed"));
        assert!(!wire.healthy);
        assert_eq!(wire.detail, "listener failed");
    }
}
// ---------------------------------------------------------------------
// Hardware inventory conversions (Task 4).
// ---------------------------------------------------------------------

/// Map a hardware capability status onto the wire enum.
pub fn hardware_status(status: gguf_engine_hardware::CapabilityStatus) -> pb::CapabilityStatus {
    use gguf_engine_hardware::CapabilityStatus as Hw;
    match status {
        Hw::Supported => pb::CapabilityStatus::Supported,
        Hw::Experimental => pb::CapabilityStatus::Experimental,
        Hw::Unsupported => pb::CapabilityStatus::Unsupported,
        Hw::Unknown => pb::CapabilityStatus::Unknown,
    }
}

/// Build the wire hardware report from a collected snapshot.
///
/// The report carries the probe status *and its reason* next to the data, so a
/// UI can say "GPU inventory unavailable: nvidia-smi failed" instead of showing
/// an empty list that reads like "this machine has no GPU".
pub fn hardware_report(snapshot: &gguf_engine_hardware::HostSnapshot) -> pb::HardwareReport {
    pb::HardwareReport {
        status: hardware_status(snapshot.gpus.status) as i32,
        unavailable_reason: snapshot.gpus.reason.clone().unwrap_or_default(),
        cpu: Some(pb::CpuInfo {
            model: snapshot.cpu.value.model.clone(),
            physical_cores: snapshot.cpu.value.physical_cores,
            logical_cores: snapshot.cpu.value.logical_cores,
            base_frequency_mhz: snapshot.cpu.value.base_frequency_mhz,
            features: Vec::new(),
            architecture: snapshot.cpu.value.architecture.clone(),
        }),
        memory: Some(pb::MemoryInfo {
            total_bytes: snapshot.memory.value.total_bytes,
            available_bytes: snapshot.memory.value.available_bytes,
            unified_with_gpu: snapshot.memory.value.unified_with_gpu,
        }),
        storage: snapshot
            .storage
            .value
            .iter()
            .map(|volume| pb::StorageInfo {
                mount_point: volume.mount_point.clone(),
                device: volume.device.clone(),
                total_bytes: volume.total_bytes,
                available_bytes: volume.available_bytes,
                removable: volume.removable,
            })
            .collect(),
        gpus: snapshot.gpus.value.iter().map(gpu_device).collect(),
        collected_at_unix_ms: snapshot.collected_at_unix_ms,
        topology: Some(gpu_topology(&snapshot.topology)),
    }
}

/// Map one GPU to its wire form.
pub fn gpu_device(device: &gguf_engine_hardware::GpuDevice) -> pb::GpuDevice {
    pb::GpuDevice {
        id: device.id.clone(),
        name: device.name.clone(),
        vendor: device.vendor.as_str().to_owned(),
        backend: device.backend.as_str().to_owned(),
        total_vram_bytes: device.total_vram_bytes,
        free_vram_bytes: device.free_vram_bytes,
        memory_bandwidth_bytes_sec: device.memory_bandwidth_bytes_sec,
        compute_capability: device.compute_capability.clone(),
        pci_bus_id: device.pci_bus_id.clone(),
        pcie_generation: device.pcie_generation,
        pcie_lanes: device.pcie_lanes,
        numa_node: device.numa_node,
        temperature_c: device.temperature_c,
        utilization_percent: device.utilization_percent,
        power_watts: device.power_watts,
        peer_access: device.peer_access.clone(),
        confidence: pb::CapabilityStatus::Supported as i32,
        driver_version: device.driver_version.clone().unwrap_or_default(),
    }
}

/// Map the topology.
pub fn gpu_topology(topology: &gguf_engine_hardware::GpuTopology) -> pb::GpuTopology {
    pb::GpuTopology {
        status: hardware_status(topology.status) as i32,
        unavailable_reason: topology.reason.clone().unwrap_or_default(),
        links: topology
            .links
            .iter()
            .map(|link| pb::PeerLink {
                from_device: link.from_device.clone(),
                to_device: link.to_device.clone(),
                interconnect: format!("{:?}", link.interconnect).to_lowercase(),
                p2p_detected: hardware_status(link.p2p_detected) as i32,
                p2p_supported_by_driver: hardware_status(link.p2p_supported_by_driver) as i32,
                p2p_tested: hardware_status(link.p2p_tested) as i32,
                measured_bandwidth_bytes_sec: link.measured_bandwidth_bytes_sec,
            })
            .collect(),
        budgets: Vec::new(),
        collected_at_unix_ms: 0,
    }
}

/// Map a registry revision onto the wire manifest.
pub fn manifest(record: &gguf_engine_database::RevisionRecord) -> pb::ModelManifest {
    pb::ModelManifest {
        schema_version: 1,
        model_id: record.model_id.clone(),
        revision: record.revision.clone(),
        architecture: record.architecture.clone().unwrap_or_default(),
        modalities: vec!["text".to_owned()],
        format: "gguf".to_owned(),
        files: record
            .files
            .iter()
            .map(|file| pb::ModelFile {
                path: file.path.clone(),
                role: file.role.clone(),
                required: file.required,
                size_bytes: u64::try_from(file.size_bytes).unwrap_or(0),
                sha256: file.sha256.clone(),
            })
            .collect(),
        runtime_requirements: vec!["llama.cpp".to_owned()],
        components: Vec::new(),
        license: record.license.clone(),
    }
}

#[cfg(test)]
mod hardware_tests {
    use super::*;
    use gguf_engine_hardware::{
        CapabilityStatus as Hw, ComputeBackend, CpuInfo, GpuDevice, GpuTopology, GpuVendor,
        HostSnapshot, Interconnect, MemoryInfo, PeerLink, ProbeOutcome, Provenance, StorageInfo,
    };

    fn empty_snapshot() -> HostSnapshot {
        let provenance = Provenance::new("test", 1);
        HostSnapshot {
            cpu: ProbeOutcome::ok(
                CpuInfo {
                    model: "Test CPU".into(),
                    physical_cores: 8,
                    logical_cores: 16,
                    base_frequency_mhz: 3200,
                    architecture: "x86_64".into(),
                },
                provenance.clone(),
            ),
            memory: ProbeOutcome::ok(
                MemoryInfo {
                    total_bytes: 1024,
                    available_bytes: 512,
                    unified_with_gpu: false,
                },
                provenance.clone(),
            ),
            storage: ProbeOutcome::ok(
                vec![StorageInfo {
                    mount_point: "C:".into(),
                    device: "disk".into(),
                    filesystem: "ntfs".into(),
                    total_bytes: 100,
                    available_bytes: 50,
                    removable: false,
                }],
                provenance.clone(),
            ),
            gpus: ProbeOutcome::ok(Vec::new(), provenance),
            topology: GpuTopology {
                links: Vec::new(),
                reason: None,
                status: Hw::Supported,
            },
            collected_at_unix_ms: 42,
        }
    }

    #[test]
    fn report_carries_the_probe_status_and_reason() {
        let mut snapshot = empty_snapshot();
        snapshot.gpus = ProbeOutcome::unavailable(
            Hw::Unsupported,
            "no supported GPU was detected (CPU-only operation)",
        );

        let report = hardware_report(&snapshot);
        assert_eq!(report.status, pb::CapabilityStatus::Unsupported as i32);
        assert!(report.unavailable_reason.contains("CPU-only"));
        assert_eq!(report.cpu.as_ref().unwrap().logical_cores, 16);
        assert_eq!(report.memory.as_ref().unwrap().total_bytes, 1024);
        assert_eq!(report.collected_at_unix_ms, 42);
    }

    #[test]
    fn gpu_fields_survive_the_conversion_including_absent_ones() {
        let device = GpuDevice {
            id: "0000:01:00.0".into(),
            name: "RTX 4090".into(),
            vendor: GpuVendor::Nvidia,
            backend: ComputeBackend::Cuda,
            total_vram_bytes: Some(24 * 1024 * 1024 * 1024),
            free_vram_bytes: Some(20 * 1024 * 1024 * 1024),
            memory_bandwidth_bytes_sec: None,
            compute_capability: Some("8.9".into()),
            pci_bus_id: Some("0000:01:00.0".into()),
            pcie_generation: Some(4),
            pcie_lanes: Some(16),
            numa_node: None,
            temperature_c: Some(52.0),
            utilization_percent: Some(8.0),
            power_watts: Some(21.5),
            peer_access: Vec::new(),
            driver_version: Some("610.88".into()),
        };

        let wire = gpu_device(&device);
        assert_eq!(wire.vendor, "nvidia");
        assert_eq!(wire.backend, "cuda");
        assert_eq!(wire.total_vram_bytes, device.total_vram_bytes);
        assert_eq!(wire.free_vram_bytes, device.free_vram_bytes);
        assert_eq!(wire.memory_bandwidth_bytes_sec, None, "absent stays absent");
        assert_eq!(wire.compute_capability.as_deref(), Some("8.9"));
        assert_eq!(wire.driver_version, "610.88");
    }

    #[test]
    fn topology_keeps_p2p_tristate_distinct() {
        let topology = GpuTopology {
            links: vec![PeerLink {
                from_device: "a".into(),
                to_device: "b".into(),
                interconnect: Interconnect::PcieAcrossNuma,
                p2p_detected: Hw::Unsupported,
                p2p_supported_by_driver: Hw::Unsupported,
                p2p_tested: Hw::Unsupported,
                measured_bandwidth_bytes_sec: None,
            }],
            reason: Some("nvidia-smi topo -m failed".into()),
            status: Hw::Unknown,
        };

        let wire = gpu_topology(&topology);
        assert_eq!(wire.status, pb::CapabilityStatus::Unknown as i32);
        assert!(wire.unavailable_reason.contains("topo"));
        assert_eq!(wire.links.len(), 1);
        assert_eq!(
            wire.links[0].p2p_tested,
            pb::CapabilityStatus::Unsupported as i32
        );
    }
}
