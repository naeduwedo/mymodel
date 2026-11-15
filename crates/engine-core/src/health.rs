//! Health reporting (spec §3.3: "健康检查 / health check").

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gguf_engine_event_bus::{BusStats, CoreState};
use serde::{Deserialize, Serialize};

/// Health of one named subsystem component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentHealth {
    /// Component name, e.g. `api`, `database`, `scheduler`.
    pub name: String,
    /// Whether the component is working.
    pub healthy: bool,
    /// Explanation when unhealthy (never a stack trace).
    pub detail: Option<String>,
}

impl ComponentHealth {
    /// A healthy component.
    pub fn healthy(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            healthy: true,
            detail: None,
        }
    }

    /// An unhealthy component with a user readable reason.
    pub fn unhealthy(name: &str, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            healthy: false,
            detail: Some(detail.into()),
        }
    }
}

/// Snapshot returned by the health endpoint and `gguf-engine doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    /// Lifecycle state at snapshot time.
    pub state: CoreState,
    /// Process uptime in milliseconds.
    pub uptime_ms: u64,
    /// UTC start time (ms since epoch).
    pub started_at_unix_ms: u64,
    /// Process id.
    pub pid: u32,
    /// Core version.
    pub version: String,
    /// Event bus counters.
    pub bus: BusStats,
    /// Per-component results.
    pub components: Vec<ComponentHealth>,
}

impl HealthReport {
    /// True when the core is serving and every component reports healthy.
    pub fn is_healthy(&self) -> bool {
        matches!(self.state, CoreState::Ready | CoreState::Degraded)
            && self.components.iter().all(|c| c.healthy)
    }

    /// Human readable one-line summary.
    pub fn summary(&self) -> String {
        let unhealthy: Vec<&str> = self
            .components
            .iter()
            .filter(|c| !c.healthy)
            .map(|c| c.name.as_str())
            .collect();
        if unhealthy.is_empty() {
            format!(
                "{} (uptime {} ms, pid {})",
                self.state.as_str(),
                self.uptime_ms,
                self.pid
            )
        } else {
            format!(
                "{} (uptime {} ms, pid {}); unhealthy: {}",
                self.state.as_str(),
                self.uptime_ms,
                self.pid,
                unhealthy.join(", ")
            )
        }
    }
}

/// Builds reports and owns the per-tick counters.
#[derive(Debug)]
pub struct HealthMonitor {
    started: Instant,
    started_at_unix_ms: u64,
    checks: AtomicU64,
}

impl HealthMonitor {
    /// Start the clock.
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            started_at_unix_ms: unix_millis(),
            checks: AtomicU64::new(0),
        }
    }

    /// Uptime in milliseconds.
    pub fn uptime_ms(&self) -> u64 {
        self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }

    /// Number of reports produced.
    pub fn checks(&self) -> u64 {
        self.checks.load(Ordering::Relaxed)
    }

    /// Assemble a report.
    pub fn report(
        &self,
        state: CoreState,
        bus: BusStats,
        components: Vec<ComponentHealth>,
    ) -> HealthReport {
        self.checks.fetch_add(1, Ordering::Relaxed);
        HealthReport {
            state,
            uptime_ms: self.uptime_ms(),
            started_at_unix_ms: self.started_at_unix_ms,
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            bus,
            components,
        }
    }
}

impl Default for HealthMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Milliseconds since the Unix epoch.
pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats() -> BusStats {
        BusStats {
            published: 1,
            dropped: 0,
            subscribers: 1,
            capacity: 8,
            last_seq: 1,
        }
    }

    #[test]
    fn healthy_when_ready_and_all_components_ok() {
        let monitor = HealthMonitor::new();
        let report = monitor.report(
            CoreState::Ready,
            stats(),
            vec![
                ComponentHealth::healthy("api"),
                ComponentHealth::healthy("database"),
            ],
        );
        assert!(report.is_healthy());
        assert!(report.summary().starts_with("ready"));
        assert_eq!(report.pid, std::process::id());
        assert_eq!(report.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn unhealthy_when_a_component_fails() {
        let monitor = HealthMonitor::new();
        let report = monitor.report(
            CoreState::Degraded,
            stats(),
            vec![ComponentHealth::unhealthy("database", "disk full")],
        );
        assert!(!report.is_healthy());
        assert!(report.summary().contains("unhealthy: database"));
    }

    #[test]
    fn not_healthy_before_ready() {
        let monitor = HealthMonitor::new();
        let report = monitor.report(CoreState::Starting, stats(), vec![]);
        assert!(!report.is_healthy());
    }

    #[test]
    fn monitor_counts_checks() {
        let monitor = HealthMonitor::new();
        assert_eq!(monitor.checks(), 0);
        monitor.report(CoreState::Ready, stats(), vec![]);
        monitor.report(CoreState::Ready, stats(), vec![]);
        assert_eq!(monitor.checks(), 2);
    }

    #[test]
    fn report_round_trips_through_json() {
        let monitor = HealthMonitor::new();
        let report = monitor.report(
            CoreState::Ready,
            stats(),
            vec![ComponentHealth::healthy("api")],
        );
        let json = serde_json::to_string(&report).unwrap();
        let back: HealthReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }
}
