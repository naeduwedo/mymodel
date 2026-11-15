//! Hardware inventory value types (spec §4.2, §5.2, §5.3).
//!
//! Two rules shape every type in this module:
//!
//! 1. **Unknown is a value.** A probe that cannot determine something says so,
//!    with a reason, instead of returning a zero that reads like a measurement
//!    (spec §5.3: P2P must distinguish detected / driver-supported / tested).
//! 2. **Every number carries its provenance.** "24 GiB" means nothing unless a
//!    reader knows it came from NVML at a given time; a UI that mixes sources
//!    silently produces numbers nobody can defend in a bug report.

use serde::{Deserialize, Serialize};

/// Whether a capability or measurement is actually available.
///
/// Mirrors `gguf.v1.CapabilityStatus`. `Unknown` is deliberately distinct from
/// `Unsupported`: the first means "we could not tell", the second means "we
/// asked and the answer was no". Collapsing them is how products end up telling
/// users their hardware is broken when a tool is simply missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// Verified on this machine.
    Supported,
    /// Implemented but not yet validated against real hardware.
    Experimental,
    /// The platform or driver cannot provide this.
    Unsupported,
    /// Present but not validated on this machine.
    ///
    /// The default: anything not explicitly established is unknown, never
    /// silently "supported" or "unsupported".
    #[default]
    Unknown,
}

impl CapabilityStatus {
    /// Stable text form used in the snapshot and the CLI.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Experimental => "experimental",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }
}

/// Where a value came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// Probe identifier, e.g. `nvidia-smi`, `sysinfo`, `system_profiler`.
    pub probe: String,
    /// Version string reported by that tool, when it has one.
    pub version: Option<String>,
    /// UTC epoch milliseconds when the value was collected.
    pub collected_at_unix_ms: u64,
}

impl Provenance {
    /// Build provenance for a probe that has no version to report.
    pub fn new(probe: &str, collected_at_unix_ms: u64) -> Self {
        Self {
            probe: probe.to_owned(),
            version: None,
            collected_at_unix_ms,
        }
    }

    /// Attach a tool version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// One-line description for logs and the CLI.
    pub fn describe(&self) -> String {
        match &self.version {
            Some(version) => format!("{} {}", self.probe, version),
            None => self.probe.clone(),
        }
    }
}

/// Result of one probe: a status, an explanation, and the data when it worked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeOutcome<T> {
    /// Whether the probe produced trustworthy data.
    pub status: CapabilityStatus,
    /// Why the status is not `Supported`; empty when it is.
    pub reason: Option<String>,
    /// The data, when there is any.
    pub value: T,
    /// Where the data came from.
    pub provenance: Option<Provenance>,
}

impl<T: Default> ProbeOutcome<T> {
    /// A probe that could not run.
    pub fn unavailable(status: CapabilityStatus, reason: impl Into<String>) -> Self {
        debug_assert_ne!(status, CapabilityStatus::Supported);
        Self {
            status,
            reason: Some(reason.into()),
            value: T::default(),
            provenance: None,
        }
    }

    /// A probe that succeeded.
    pub fn ok(value: T, provenance: Provenance) -> Self {
        Self {
            status: CapabilityStatus::Supported,
            reason: None,
            value,
            provenance: Some(provenance),
        }
    }
}

impl<T> ProbeOutcome<T> {
    /// True when the caller can trust `value`.
    pub fn is_usable(&self) -> bool {
        matches!(
            self.status,
            CapabilityStatus::Supported | CapabilityStatus::Experimental
        )
    }

    /// Map the value while keeping status, reason and provenance.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> ProbeOutcome<U> {
        ProbeOutcome {
            status: self.status,
            reason: self.reason,
            value: f(self.value),
            provenance: self.provenance,
        }
    }
}

/// CPU description.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CpuInfo {
    /// Marketing name, when the OS exposes it.
    pub model: String,
    /// Physical cores.
    pub physical_cores: u32,
    /// Logical processors.
    pub logical_cores: u32,
    /// Nominal clock in MHz.
    pub base_frequency_mhz: u64,
    /// Architecture, e.g. `x86_64`, `aarch64`.
    pub architecture: String,
}

/// Host memory description.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MemoryInfo {
    /// Total physical RAM in bytes.
    pub total_bytes: u64,
    /// Currently available RAM in bytes.
    pub available_bytes: u64,
    /// True when the GPU shares this memory (Apple Silicon, some APUs).
    ///
    /// When true, consumers must **not** report a separate VRAM budget: the
    /// same bytes back both (spec §5.2).
    pub unified_with_gpu: bool,
}

impl MemoryInfo {
    /// Bytes currently in use.
    pub fn used_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.available_bytes)
    }
}

/// One mounted volume.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StorageInfo {
    /// Mount point or drive letter.
    pub mount_point: String,
    /// Device name when known.
    pub device: String,
    /// Filesystem name when known.
    pub filesystem: String,
    /// Total capacity in bytes.
    pub total_bytes: u64,
    /// Free space in bytes.
    pub available_bytes: u64,
    /// Removable media (a USB drive disappears between two snapshots).
    pub removable: bool,
}

/// GPU vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuVendor {
    /// NVIDIA.
    Nvidia,
    /// AMD.
    Amd,
    /// Apple.
    Apple,
    /// Intel.
    Intel,
    /// Unrecognised.
    Unknown,
}

impl GpuVendor {
    /// Stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nvidia => "nvidia",
            Self::Amd => "amd",
            Self::Apple => "apple",
            Self::Intel => "intel",
            Self::Unknown => "unknown",
        }
    }

    /// Guess the vendor from a device name (used only when the probe cannot say).
    pub fn from_name(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        if lower.contains("nvidia")
            || lower.contains("geforce")
            || lower.contains("quadro")
            || lower.contains("tesla")
        {
            Self::Nvidia
        } else if lower.contains("amd") || lower.contains("radeon") || lower.contains("instinct") {
            Self::Amd
        } else if lower.contains("apple") {
            Self::Apple
        } else if lower.contains("intel") || lower.contains("arc") {
            Self::Intel
        } else {
            Self::Unknown
        }
    }
}

/// Compute backend a device can actually use (spec §2.4, §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeBackend {
    /// NVIDIA CUDA.
    Cuda,
    /// AMD ROCm.
    Rocm,
    /// Apple Metal.
    Metal,
    /// Vulkan (not a declared backend yet; recorded, never advertised).
    Vulkan,
    /// DirectML (same caveat as Vulkan).
    DirectMl,
    /// CPU only.
    Cpu,
    /// Not determined.
    Unknown,
}

impl ComputeBackend {
    /// Stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Rocm => "rocm",
            Self::Metal => "metal",
            Self::Vulkan => "vulkan",
            Self::DirectMl => "directml",
            Self::Cpu => "cpu",
            Self::Unknown => "unknown",
        }
    }
}

/// One GPU (spec §5.2). Every optional field may be absent; absence is data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuDevice {
    /// Stable identifier: the PCI bus id when known, else the probe index.
    pub id: String,
    /// Marketing name.
    pub name: String,
    /// Vendor.
    pub vendor: GpuVendor,
    /// Backend this device can be driven with.
    pub backend: ComputeBackend,
    /// Total VRAM; `None` for unified-memory devices, where there is no
    /// separate pool to report.
    pub total_vram_bytes: Option<u64>,
    /// Free VRAM at probe time.
    pub free_vram_bytes: Option<u64>,
    /// Peak memory bandwidth, when the driver exposes it.
    pub memory_bandwidth_bytes_sec: Option<u64>,
    /// Compute capability string, e.g. `8.9` for NVIDIA.
    pub compute_capability: Option<String>,
    /// PCI bus id, e.g. `0000:01:00.0`.
    pub pci_bus_id: Option<String>,
    /// PCIe generation the link currently negotiated.
    pub pcie_generation: Option<u32>,
    /// PCIe width the link currently negotiated.
    pub pcie_lanes: Option<u32>,
    /// NUMA node, when the platform reports one.
    pub numa_node: Option<u32>,
    /// Temperature in Celsius.
    pub temperature_c: Option<f32>,
    /// Utilisation percent.
    pub utilization_percent: Option<f32>,
    /// Board power draw in watts.
    pub power_watts: Option<f32>,
    /// Device ids this device can address peer-to-peer, **verified only**.
    pub peer_access: Vec<String>,
    /// Driver version reported by the probe.
    pub driver_version: Option<String>,
}

impl GpuDevice {
    /// Free VRAM as a fraction of total, when both are known.
    pub fn free_ratio(&self) -> Option<f64> {
        match (self.free_vram_bytes, self.total_vram_bytes) {
            (Some(free), Some(total)) if total > 0 => Some(free as f64 / total as f64),
            _ => None,
        }
    }
}

/// Interconnect between two devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interconnect {
    /// Same physical card.
    SameDevice,
    /// NVLink bridge.
    NvLink,
    /// PCIe switch.
    PcieSwitch,
    /// Plain PCIe, host-mediated.
    Pcie,
    /// Different NUMA nodes over PCIe.
    PcieAcrossNuma,
    /// Known to be absent.
    None,
    /// Not determined.
    Unknown,
}

impl Interconnect {
    /// True when the link is a direct device-to-device path.
    pub fn is_direct(self) -> bool {
        matches!(self, Self::SameDevice | Self::NvLink)
    }
}

/// Peer-to-peer state for a device pair (spec §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerLink {
    /// Source device id.
    pub from_device: String,
    /// Target device id.
    pub to_device: String,
    /// Interconnect type.
    pub interconnect: Interconnect,
    /// The driver reports the pair as P2P capable.
    pub p2p_detected: CapabilityStatus,
    /// The driver says the topology allows P2P.
    pub p2p_supported_by_driver: CapabilityStatus,
    /// We actually transferred over P2P and it worked.
    ///
    /// Always `Unsupported` until a benchmark exists (Task 7): "detected" is
    /// not "tested", and the UI must not conflate them.
    pub p2p_tested: CapabilityStatus,
    /// Measured bandwidth, only when `p2p_tested` is `Supported`.
    pub measured_bandwidth_bytes_sec: Option<u64>,
}

/// GPU topology as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GpuTopology {
    /// Links between devices; empty when there is one device or no data.
    pub links: Vec<PeerLink>,
    /// Why the topology could not be determined, when that is the case.
    pub reason: Option<String>,
    /// Status of the topology probe as a whole.
    pub status: CapabilityStatus,
}

/// Everything one inventory run collected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostSnapshot {
    /// CPU.
    pub cpu: ProbeOutcome<CpuInfo>,
    /// Host memory.
    pub memory: ProbeOutcome<MemoryInfo>,
    /// Storage.
    pub storage: ProbeOutcome<Vec<StorageInfo>>,
    /// GPUs.
    pub gpus: ProbeOutcome<Vec<GpuDevice>>,
    /// Topology between those GPUs.
    pub topology: GpuTopology,
    /// UTC epoch milliseconds for the whole run.
    pub collected_at_unix_ms: u64,
}

impl HostSnapshot {
    /// Total VRAM across all devices that report a discrete pool.
    ///
    /// Deliberately *not* presented as usable capacity: spec §5.4 forbids
    /// summing VRAM into one pool. It is a display total, nothing more.
    pub fn total_vram_bytes(&self) -> u64 {
        self.gpus
            .value
            .iter()
            .map(|gpu| gpu.total_vram_bytes.unwrap_or(0))
            .sum()
    }

    /// True when any GPU shares memory with the CPU.
    pub fn has_unified_memory(&self) -> bool {
        self.memory.value.unified_with_gpu
    }

    /// Every status that is not `Supported`, for the UI to explain.
    pub fn gaps(&self) -> Vec<(&'static str, &str)> {
        let mut gaps = Vec::new();
        if !self.cpu.is_usable() {
            gaps.push(("cpu", self.cpu.reason.as_deref().unwrap_or("unknown")));
        }
        if !self.memory.is_usable() {
            gaps.push(("memory", self.memory.reason.as_deref().unwrap_or("unknown")));
        }
        if !self.storage.is_usable() {
            gaps.push((
                "storage",
                self.storage.reason.as_deref().unwrap_or("unknown"),
            ));
        }
        if !self.gpus.is_usable() {
            gaps.push(("gpus", self.gpus.reason.as_deref().unwrap_or("unknown")));
        }
        if let Some(reason) = self.topology.reason.as_deref() {
            gaps.push(("topology", reason));
        }
        gaps
    }
}

/// UTC epoch milliseconds.
pub fn unix_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}
