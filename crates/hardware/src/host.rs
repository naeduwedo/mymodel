//! CPU, memory and storage probes.
//!
//! Implemented with `sysinfo`, which is cross-platform and safe. Platform
//! specifics that `sysinfo` cannot answer (Apple unified memory topology) are
//! handled where they belong, in the platform GPU probe, and surfaced here
//! through [`MemoryInfo::unified_with_gpu`].

use sysinfo::{Disks, System};

use crate::types::{
    CapabilityStatus, CpuInfo, MemoryInfo, ProbeOutcome, Provenance, StorageInfo, unix_millis,
};

/// Probe the CPU.
pub fn cpu() -> ProbeOutcome<CpuInfo> {
    let mut system = System::new();
    system.refresh_cpu_all();

    let cpus = system.cpus();
    if cpus.is_empty() {
        return ProbeOutcome::unavailable(
            CapabilityStatus::Unsupported,
            "the OS reported no CPU information",
        );
    }

    let logical_cores = u32::try_from(cpus.len()).unwrap_or(u32::MAX);
    let base_frequency_mhz = cpus.iter().map(|cpu| cpu.frequency()).max().unwrap_or(0);
    let model = cpus
        .first()
        .map(|cpu| cpu.brand().trim().to_owned())
        .unwrap_or_default();

    // `sysinfo` exposes logical processors only. Reporting `logical` as
    // `physical` would inflate core counts in every later budget, so the
    // physical count stays 0 (= not known) rather than being guessed.
    ProbeOutcome::ok(
        CpuInfo {
            model,
            // physical_core_count is a static in sysinfo 0.37: it asks the OS
            // directly instead of reading a refreshed System.
            physical_cores: u32::try_from(System::physical_core_count().unwrap_or(0)).unwrap_or(0),
            logical_cores,
            base_frequency_mhz,
            architecture: std::env::consts::ARCH.to_owned(),
        },
        Provenance::new("sysinfo", unix_millis()),
    )
}

/// Probe host memory.
pub fn memory() -> ProbeOutcome<MemoryInfo> {
    let mut system = System::new();
    system.refresh_memory();

    let total = system.total_memory();
    if total == 0 {
        return ProbeOutcome::unavailable(
            CapabilityStatus::Unknown,
            "the OS reported zero total memory",
        );
    }

    // Apple Silicon shares one pool between CPU and GPU; reporting it as both
    // RAM *and* VRAM would double-count it (spec §5.2).
    let unified = cfg!(target_os = "macos") && cfg!(target_arch = "aarch64");

    ProbeOutcome::ok(
        MemoryInfo {
            total_bytes: total,
            available_bytes: system.available_memory(),
            unified_with_gpu: unified,
        },
        Provenance::new("sysinfo", unix_millis()),
    )
}

/// Probe mounted storage.
pub fn storage() -> ProbeOutcome<Vec<StorageInfo>> {
    let disks = Disks::new_with_refreshed_list();
    if disks.list().is_empty() {
        return ProbeOutcome::unavailable(
            CapabilityStatus::Unsupported,
            "the OS reported no mounted volumes",
        );
    }

    let mut volumes: Vec<StorageInfo> = disks
        .list()
        .iter()
        .map(|disk| StorageInfo {
            mount_point: disk.mount_point().display().to_string(),
            device: disk.name().to_string_lossy().into_owned(),
            filesystem: disk.file_system().to_string_lossy().into_owned(),
            total_bytes: disk.total_space(),
            available_bytes: disk.available_space(),
            removable: disk.is_removable(),
        })
        .collect();

    // Stable order so two probes of the same machine produce the same document.
    volumes.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));

    ProbeOutcome::ok(volumes, Provenance::new("sysinfo", unix_millis()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_is_reported_on_any_machine() {
        let outcome = cpu();
        assert!(outcome.is_usable(), "reason: {:?}", outcome.reason);
        assert!(outcome.value.logical_cores > 0);
        assert!(!outcome.value.architecture.is_empty());
    }

    #[test]
    fn physical_cores_are_never_inflated_to_logical_cores() {
        let outcome = cpu();
        let cpu = outcome.value;
        assert!(
            cpu.physical_cores <= cpu.logical_cores,
            "physical {} > logical {}",
            cpu.physical_cores,
            cpu.logical_cores
        );
    }

    #[test]
    fn memory_totals_are_plausible() {
        let outcome = memory();
        assert!(outcome.is_usable(), "reason: {:?}", outcome.reason);
        let memory = outcome.value;
        assert!(
            memory.total_bytes > 512 * 1024 * 1024,
            "too small: {}",
            memory.total_bytes
        );
        assert!(memory.available_bytes <= memory.total_bytes);
        // Unified memory is a macOS-on-Apple-Silicon property only.
        assert_eq!(
            memory.unified_with_gpu,
            cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")
        );
    }

    #[test]
    fn storage_lists_at_least_one_volume_in_stable_order() {
        let outcome = storage();
        assert!(outcome.is_usable(), "reason: {:?}", outcome.reason);
        let volumes = &outcome.value;
        assert!(!volumes.is_empty());
        let mut sorted = volumes.clone();
        sorted.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
        assert_eq!(volumes, &sorted);
        for volume in volumes {
            assert!(volume.available_bytes <= volume.total_bytes);
        }
    }
}
