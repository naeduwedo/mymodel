//! Memory tiers and their budgets (spec 6).
//!
//! The tier model is the whole point: NVMe is a **cold** tier, never an
//! extension of VRAM (ADR-006). This module refuses to pretend otherwise, and
//! the type names make the distinction hard to ignore.

use serde::{Deserialize, Serialize};

/// The storage tier a byte lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// L0: GPU VRAM. Fast, small, budgeted per device.
    GpuVram,
    /// L1: host RAM. Slower, larger, shared with the OS.
    HostRam,
    /// L2: NVMe. A cold data source, not addressable like memory.
    Nvme,
}

impl Tier {
    /// Stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GpuVram => "gpu_vram",
            Self::HostRam => "host_ram",
            Self::Nvme => "nvme",
        }
    }

    /// True when bytes in this tier can be read at memory-like latency.
    ///
    /// NVMe is deliberately false: treating it as memory is the mistake
    /// ADR-006 exists to prevent.
    pub fn is_addressable_memory(self) -> bool {
        !matches!(self, Self::Nvme)
    }
}

/// A tier and the budget computed for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierBudget {
    /// Which tier.
    pub tier: Tier,
    /// Total capacity observed.
    pub total: u64,
    /// Bytes currently free.
    pub available: u64,
    /// Reserve kept free for the OS and other applications.
    pub reserve: u64,
    /// Bytes this engine may use.
    pub usable: u64,
}

impl TierBudget {
    /// Compute `usable = available - reserve`, saturating at zero.
    pub fn compute(tier: Tier, total: u64, available: u64, reserve: u64) -> Self {
        Self {
            tier,
            total,
            available,
            reserve,
            usable: available.saturating_sub(reserve),
        }
    }
}

/// Memory pressure, as the UI and the scheduler see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pressure {
    /// Plenty of headroom.
    Nominal,
    /// Usable RAM is getting low; new loads should be refused or degraded.
    Elevated,
    /// The reserve is being eaten; stop prefetching immediately.
    Critical,
}

impl Pressure {
    /// Classify from the fraction of the reserve still intact.
    ///
    /// `available` is what the OS reports; `reserve` is the floor we promised
    /// to leave free. Crossing that floor is critical, not merely elevated.
    pub fn classify(available: u64, reserve: u64) -> Self {
        if reserve == 0 {
            return Self::Nominal;
        }
        if available <= reserve {
            Self::Critical
        } else if available <= reserve + reserve / 2 {
            Self::Elevated
        } else {
            Self::Nominal
        }
    }

    /// Stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::Elevated => "elevated",
            Self::Critical => "critical",
        }
    }

    /// Whether new model loads may start.
    pub fn allows_new_load(self) -> bool {
        matches!(self, Self::Nominal)
    }
}

/// A prefetch request, bounded by policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrefetchRequest {
    /// Bytes the caller wants to read ahead.
    pub bytes: u64,
}

/// Why a prefetch was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PrefetchError(pub String);

/// Decide how much of a prefetch request to honour.
///
/// Returns the granted byte count, which may be smaller than requested. Under
/// `Critical` pressure nothing is granted: prefetch is the first thing to stop,
/// because it competes with the OS for the reserve.
pub fn grant_prefetch(
    request: PrefetchRequest,
    cap: u64,
    pressure: Pressure,
    already_in_flight: u64,
) -> Result<u64, PrefetchError> {
    if !pressure.allows_new_load() {
        return Err(PrefetchError(format!(
            "prefetch refused: memory pressure is {}",
            pressure.as_str()
        )));
    }

    let headroom = cap.saturating_sub(already_in_flight);
    if headroom == 0 {
        return Err(PrefetchError(format!(
            "prefetch refused: {:.1} GiB already in flight, cap is {:.1} GiB",
            gib(already_in_flight),
            gib(cap)
        )));
    }

    Ok(request.bytes.min(headroom))
}

/// Read live host memory figures.
pub fn host_memory() -> (u64, u64) {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    (system.total_memory(), system.available_memory())
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}
