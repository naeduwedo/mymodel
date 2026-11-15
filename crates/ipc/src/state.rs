//! State shared by every request handler.

use std::sync::Arc;
use std::time::Instant;

use gguf_engine_config::{Config, Issue};
use gguf_engine_database::Database;
use gguf_engine_event_bus::{CoreState as LifecycleState, EventBus};
use gguf_engine_protocol::pb;
use parking_lot::RwLock;

/// A cloneable view of the core's lifecycle, safe to read from any request.
#[derive(Debug)]
pub struct LifecycleHandle {
    state: RwLock<LifecycleState>,
    started: Instant,
}

impl LifecycleHandle {
    /// Create a handle in the given state.
    pub fn new(state: LifecycleState) -> Self {
        Self {
            state: RwLock::new(state),
            started: Instant::now(),
        }
    }

    /// Current state.
    pub fn current(&self) -> LifecycleState {
        *self.state.read()
    }

    /// Update the state (the lifecycle owner calls this on every transition).
    pub fn set(&self, state: LifecycleState) {
        *self.state.write() = state;
    }

    /// Uptime in milliseconds.
    pub fn uptime_ms(&self) -> u64 {
        self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
}

/// Everything a service needs to answer a request.
#[derive(Clone, Debug)]
pub struct CoreState {
    /// Typed event bus (also feeds the event stream service).
    pub bus: EventBus,
    /// Lifecycle view.
    pub lifecycle: Arc<LifecycleHandle>,
    /// Registry database.
    pub database: Database,
    /// Process start time (UTC ms) used in health reports.
    pub started_at_unix_ms: u64,
    /// Random identifier for this core instance.
    pub instance_id: String,
    /// Protocol version this build speaks.
    pub protocol_version: u32,
    /// Feature matrix reported to clients.
    pub features: Vec<gguf_engine_protocol::Feature>,
    /// Effective configuration (what the UI shells display in Settings).
    pub config: Config,
    /// Validation findings for that configuration.
    pub config_issues: Vec<Issue>,
    /// Path of the configuration file in effect.
    pub config_file: String,
    /// Whether that file exists (otherwise the defaults are in use).
    pub config_file_exists: bool,
    /// Data root in use.
    pub data_root: String,
    /// Directory holding downloaded model files.
    ///
    /// Stored rather than derived from `data_root` because the models directory
    /// is configurable; deriving it would silently point the inference service at
    /// the wrong place for anyone who moved it.
    pub models_dir: String,
    /// Endpoint the core is serving on.
    pub endpoint: String,
    /// Cached hardware inventory, shared by `HardwareService` and the Overview
    /// snapshot so both describe the same probe result.
    pub hardware: Arc<crate::services::HardwareCache>,
}

impl CoreState {
    /// Build the wire feature list.
    pub fn wire_features(&self) -> Vec<pb::Feature> {
        self.features.iter().map(|f| f.to_pb()).collect()
    }

    /// Hardware summary for the Overview page.
    ///
    /// Returns tiles that state the *probe result*, not a promise: when the GPU
    /// probe failed the tile says so with its reason rather than "0 GPUs".
    pub async fn hardware_tiles(&self) -> Vec<pb::UiTile> {
        let snapshot = self.hardware.get().await;
        let mut tiles = Vec::new();

        let cpu = &snapshot.cpu.value;
        tiles.push(pb::UiTile {
            id: "hw.cpu".into(),
            label: "CPU".into(),
            value: if cpu.model.is_empty() {
                format!("{} cores", cpu.logical_cores)
            } else {
                cpu.model.clone()
            },
            severity: if snapshot.cpu.is_usable() {
                pb::UiSeverity::Ok as i32
            } else {
                pb::UiSeverity::Warning as i32
            },
            detail: format!(
                "{} physical / {} logical cores, {}",
                cpu.physical_cores, cpu.logical_cores, cpu.architecture
            ),
        });

        let memory = &snapshot.memory.value;
        tiles.push(pb::UiTile {
            id: "hw.memory".into(),
            label: "System RAM".into(),
            value: gguf_engine_protocol::snapshot::format_bytes(memory.total_bytes),
            severity: pb::UiSeverity::Ok as i32,
            detail: format!(
                "{} available{}",
                gguf_engine_protocol::snapshot::format_bytes(memory.available_bytes),
                if memory.unified_with_gpu {
                    ", unified with the GPU"
                } else {
                    ""
                }
            ),
        });

        let gpu_count = snapshot.gpus.value.len();
        let gpu_tile = pb::UiTile {
            id: "hw.gpu".into(),
            label: "GPUs".into(),
            value: if gpu_count == 0 {
                "none detected".to_owned()
            } else {
                format!(
                    "{gpu_count} device(s), {} VRAM",
                    gguf_engine_protocol::snapshot::format_bytes(snapshot.total_vram_bytes())
                )
            },
            severity: match snapshot.gpus.status {
                gguf_engine_hardware::CapabilityStatus::Supported => pb::UiSeverity::Ok as i32,
                gguf_engine_hardware::CapabilityStatus::Experimental => {
                    pb::UiSeverity::Warning as i32
                }
                gguf_engine_hardware::CapabilityStatus::Unknown => pb::UiSeverity::Warning as i32,
                gguf_engine_hardware::CapabilityStatus::Unsupported => {
                    pb::UiSeverity::Disabled as i32
                }
            },
            detail: snapshot.gpus.reason.clone().unwrap_or_else(|| {
                snapshot
                    .gpus
                    .value
                    .iter()
                    .map(|gpu| gpu.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            }),
        };
        tiles.push(gpu_tile);

        if let Some(reason) = &snapshot.topology.reason {
            tiles.push(pb::UiTile {
                id: "hw.topology".into(),
                label: "GPU topology".into(),
                value: "unknown".into(),
                severity: pb::UiSeverity::Warning as i32,
                detail: reason.clone(),
            });
        }

        tiles
    }

    /// Collect the presentation snapshot shared by all three native UIs.
    ///
    /// Kept here (not in a shell) so Overview and Settings cannot drift between
    /// Windows, Linux and macOS.
    pub fn snapshot(&self, schema_version: i64, sqlite_version: String) -> pb::UiSnapshot {
        let input = gguf_engine_protocol::snapshot::SnapshotInput {
            state: self.lifecycle.current(),
            uptime_ms: self.lifecycle.uptime_ms(),
            pid: std::process::id(),
            core_version: env!("CARGO_PKG_VERSION").to_owned(),
            endpoint: self.endpoint.clone(),
            data_root: self.data_root.clone(),
            schema_version,
            sqlite_version,
            bus: self.bus.stats(),
            config: self.config.clone(),
            config_file: self.config_file.clone(),
            config_file_exists: self.config_file_exists,
            generated_at_local: gguf_engine_protocol::snapshot::now_local(),
        };

        let events: Vec<pb::UiEventLine> = self
            .bus
            .replay_after(0)
            .iter()
            .filter_map(|event| {
                gguf_engine_protocol::snapshot::event_line(
                    event.seq,
                    event.at_unix_ms,
                    &event.payload,
                )
            })
            .collect();

        gguf_engine_protocol::snapshot::snapshot(&input, &self.config_issues, &events)
    }
}
