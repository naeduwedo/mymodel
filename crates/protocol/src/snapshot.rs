//! Building the presentation snapshot shared by the three native UIs.
//!
//! Spec §4 requires the same information architecture and terminology in all
//! three shells. That is only achievable if the *core* decides what a value
//! means; a shell that formats its own strings drifts on the first rename.
//!
//! Everything here is pure: inputs are the live core state and the effective
//! configuration, the output is a snapshot message. That makes it testable
//! without a daemon, a UI toolkit or a display - the important part, since only
//! one of the three toolkits can even be compiled on a given machine.

use gguf_engine_config::{Config, Issue, Severity as ConfigSeverity};
use gguf_engine_event_bus::{BusStats, CoreState, EventPayload, NoticeLevel};

use crate::features::{FeatureStatus, features};
use crate::pb;

/// Version of the snapshot payload. Bump when a shell would break.
pub const SNAPSHOT_VERSION: u32 = 1;

/// Everything the snapshot needs about the running core.
#[derive(Debug, Clone)]
pub struct SnapshotInput {
    /// Lifecycle state.
    pub state: CoreState,
    /// Uptime in milliseconds.
    pub uptime_ms: u64,
    /// Process id.
    pub pid: u32,
    /// Core version string.
    pub core_version: String,
    /// IPC endpoint the core is serving on.
    pub endpoint: String,
    /// Data root in use.
    pub data_root: String,
    /// Database schema version.
    pub schema_version: i64,
    /// Linked SQLite version.
    pub sqlite_version: String,
    /// Event bus counters.
    pub bus: BusStats,
    /// Effective configuration.
    pub config: Config,
    /// Path of the configuration file in effect.
    pub config_file: String,
    /// Whether that file exists (otherwise defaults are in use).
    pub config_file_exists: bool,
    /// Time the snapshot was produced, pre-formatted and timezone-labelled.
    pub generated_at_local: String,
}

/// Timestamp string for "now".
///
/// Deliberately UTC and explicitly labelled: the core has no idea which zone the
/// user sits in, and a bare local-looking timestamp would be a lie.
pub fn now_local() -> String {
    format!("{} UTC", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"))
}

fn severity_for_state(state: CoreState) -> pb::UiSeverity {
    match state {
        CoreState::Ready => pb::UiSeverity::Ok,
        CoreState::Starting | CoreState::ShuttingDown | CoreState::Degraded => {
            pb::UiSeverity::Warning
        }
        CoreState::Stopped => pb::UiSeverity::Error,
    }
}

fn severity_for_feature(status: FeatureStatus) -> pb::UiSeverity {
    match status {
        FeatureStatus::Supported => pb::UiSeverity::Ok,
        FeatureStatus::Experimental | FeatureStatus::Unknown => pb::UiSeverity::Warning,
        FeatureStatus::Planned | FeatureStatus::Unsupported => pb::UiSeverity::Disabled,
    }
}

fn severity_for_issue(severity: ConfigSeverity) -> pb::UiSeverity {
    match severity {
        ConfigSeverity::Error => pb::UiSeverity::Error,
        ConfigSeverity::Warning => pb::UiSeverity::Warning,
    }
}

/// Human label for a capability id, shared by every shell.
pub fn feature_label(id: &str) -> &str {
    match id {
        "ipc" => "IPC transport",
        "event_stream" => "Event stream",
        "database" => "Registry database",
        "health" => "Health checks",
        "hardware_inventory" => "Hardware inventory",
        "model_registry" => "Model registry",
        "model_download" => "Model download",
        "runtime_llama_cpp" => "llama.cpp runtime",
        "multi_gpu_scheduling" => "Multi-GPU scheduling",
        "memory_paging" => "Memory paging",
        "vision" => "Vision (image in)",
        "inference" => "Chat (streaming)",
        "multimodal_other" => "Image / video / audio / music",
        "http_api" => "HTTP / OpenAI API",
        other => other,
    }
}

/// Format an uptime the way a status line should read.
pub fn format_uptime(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let (days, rem) = (total_seconds / 86_400, total_seconds % 86_400);
    let (hours, rem) = (rem / 3600, rem % 3600);
    let (minutes, seconds) = (rem / 60, rem % 60);
    if days > 0 {
        format!("{days}d {hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }
}

/// Format a byte count with binary units.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn millis_to_hms(at_unix_ms: u64) -> String {
    let secs_of_day = (at_unix_ms / 1000) % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Turn a bus event into a display line, or drop it when it is noise.
pub fn event_line(seq: u64, at_unix_ms: u64, payload: &EventPayload) -> Option<pb::UiEventLine> {
    let (severity, text) = match payload {
        EventPayload::CoreStateChanged { from, to } => (
            severity_for_state(*to),
            format!("Core state {} -> {}", from.as_str(), to.as_str()),
        ),
        // Health ticks are a liveness signal, not user-facing history.
        EventPayload::HealthTick { .. } => return None,
        EventPayload::WorkerStarted {
            worker_id,
            runtime_id,
        } => (
            pb::UiSeverity::Ok,
            format!("Worker {worker_id} started ({runtime_id})"),
        ),
        EventPayload::WorkerStopped {
            worker_id,
            exit_code,
            reason,
        } => (
            pb::UiSeverity::Error,
            format!(
                "Worker {worker_id} stopped (exit {exit_code:?}{})",
                reason
                    .as_deref()
                    .map(|r| format!(", {r}"))
                    .unwrap_or_default()
            ),
        ),
        EventPayload::Notice { level, message } => {
            let severity = match level {
                NoticeLevel::Info => pb::UiSeverity::Ok,
                NoticeLevel::Warning => pb::UiSeverity::Warning,
                NoticeLevel::Error => pb::UiSeverity::Error,
            };
            (severity, message.clone())
        }
    };

    Some(pb::UiEventLine {
        seq,
        at_local: format!("{} UTC", millis_to_hms(at_unix_ms)),
        severity: severity as i32,
        text,
    })
}

/// Build the Overview section.
pub fn overview(input: &SnapshotInput, recent_events: &[pb::UiEventLine]) -> pb::UiOverview {
    let feature_rows: Vec<pb::UiFeatureRow> = features()
        .into_iter()
        .map(|feature| pb::UiFeatureRow {
            label: feature_label(&feature.id).to_owned(),
            status: feature.status.to_pb() as i32,
            status_text: feature.status.as_str().to_owned(),
            reason: feature.reason.unwrap_or_default(),
            id: feature.id,
            // Severity is derived once, here, so all three shells colour the
            // same row the same way.
            severity: severity_for_feature(feature.status) as i32,
        })
        .collect();

    let missing: Vec<String> = feature_rows
        .iter()
        .filter(|row| row.status == pb::CapabilityStatus::Planned as i32)
        .map(|row| row.label.clone())
        .collect();

    let tiles = vec![
        pb::UiTile {
            id: "core.state".into(),
            label: "Core state".into(),
            value: input.state.as_str().to_owned(),
            severity: severity_for_state(input.state) as i32,
            detail: if input.state == CoreState::Degraded {
                "Serving, but at least one capability is unavailable".into()
            } else {
                String::new()
            },
        },
        pb::UiTile {
            id: "core.uptime".into(),
            label: "Uptime".into(),
            value: format_uptime(input.uptime_ms),
            severity: pb::UiSeverity::Ok as i32,
            detail: String::new(),
        },
        pb::UiTile {
            id: "core.version".into(),
            label: "Core version".into(),
            value: input.core_version.clone(),
            severity: pb::UiSeverity::Ok as i32,
            detail: format!("pid {}", input.pid),
        },
        pb::UiTile {
            id: "core.endpoint".into(),
            label: "IPC endpoint".into(),
            value: input.endpoint.clone(),
            severity: pb::UiSeverity::Ok as i32,
            detail: "Local-only transport".into(),
        },
        pb::UiTile {
            id: "db.schema".into(),
            label: "Database".into(),
            value: format!("schema v{}", input.schema_version),
            severity: pb::UiSeverity::Ok as i32,
            detail: format!("SQLite {}", input.sqlite_version),
        },
        pb::UiTile {
            id: "events.bus".into(),
            label: "Event bus".into(),
            value: format!("{} published", input.bus.published),
            severity: if input.bus.dropped > 0 {
                pb::UiSeverity::Warning as i32
            } else {
                pb::UiSeverity::Ok as i32
            },
            detail: format!(
                "{} subscriber(s), {} dropped, capacity {}",
                input.bus.subscribers, input.bus.dropped, input.bus.capacity
            ),
        },
        pb::UiTile {
            id: "memory.gpu_reserve".into(),
            label: "VRAM reserve".into(),
            value: format_bytes(input.config.memory.gpu_reserve_bytes),
            severity: pb::UiSeverity::Ok as i32,
            detail: "Kept free on every GPU (config)".into(),
        },
        pb::UiTile {
            id: "features.missing".into(),
            label: "Not implemented".into(),
            value: if missing.is_empty() {
                "none".to_owned()
            } else {
                format!("{} capabilities", missing.len())
            },
            severity: if missing.is_empty() {
                pb::UiSeverity::Ok as i32
            } else {
                pb::UiSeverity::Disabled as i32
            },
            detail: missing.join(", "),
        },
    ];

    // Attention is required when something is broken *or* when the user is
    // looking at a build with missing capabilities: both are worth acting on.
    let attention_required = input.state != CoreState::Ready || !missing.is_empty();

    pb::UiOverview {
        tiles,
        features: feature_rows,
        recent_events: recent_events.to_vec(),
        attention_required,
    }
}
const READONLY: &str = "Read-only in this build; editing lands with the settings service";

fn setting(
    key: &str,
    label: &str,
    value: impl Into<String>,
    note: &str,
    severity: pb::UiSeverity,
) -> pb::UiSettingRow {
    pb::UiSettingRow {
        key: key.to_owned(),
        label: label.to_owned(),
        value: value.into(),
        source: "config".to_owned(),
        // Task 3 renders settings read-only. Reporting `editable = false` plus a
        // note is what stops a shell from drawing a control that does nothing.
        editable: false,
        note: note.to_owned(),
        severity: severity as i32,
    }
}

/// Build the Settings section.
pub fn settings(input: &SnapshotInput, issues: &[Issue]) -> pb::UiSettings {
    let ok = pb::UiSeverity::Ok;
    let mut groups = vec![
        pb::UiSettingsGroup {
            title: "Core".into(),
            rows: vec![
                setting(
                    "core.host",
                    "API host",
                    input.config.core.host.clone(),
                    READONLY,
                    ok,
                ),
                setting(
                    "core.api_port",
                    "API port",
                    input.config.core.api_port.to_string(),
                    READONLY,
                    ok,
                ),
                setting(
                    "core.grpc_port",
                    "IPC port",
                    input.config.core.grpc_port.to_string(),
                    READONLY,
                    ok,
                ),
                setting(
                    "core.log_level",
                    "Log level",
                    input.config.core.log_level.clone(),
                    READONLY,
                    ok,
                ),
                setting(
                    "core.keep_workers_after_ui_exit",
                    "Keep workers after UI exit",
                    input.config.core.keep_workers_after_ui_exit.to_string(),
                    READONLY,
                    ok,
                ),
            ],
        },
        pb::UiSettingsGroup {
            title: "Paths".into(),
            rows: vec![
                setting(
                    "data.root",
                    "Data root",
                    input.data_root.clone(),
                    READONLY,
                    ok,
                ),
                setting(
                    "paths.models",
                    "Models",
                    input.config.paths.models.clone(),
                    READONLY,
                    ok,
                ),
                setting(
                    "paths.cache",
                    "Cache",
                    input.config.paths.cache.clone(),
                    READONLY,
                    ok,
                ),
                setting(
                    "paths.runtimes",
                    "Runtimes",
                    input.config.paths.runtimes.clone(),
                    READONLY,
                    ok,
                ),
                setting(
                    "paths.logs",
                    "Logs",
                    input.config.paths.logs.clone(),
                    READONLY,
                    ok,
                ),
                setting(
                    "paths.database",
                    "Database",
                    input.config.paths.database.clone(),
                    READONLY,
                    ok,
                ),
            ],
        },
        pb::UiSettingsGroup {
            title: "Memory".into(),
            rows: vec![
                setting(
                    "memory.strategy",
                    "Strategy",
                    input.config.memory.strategy.clone(),
                    READONLY,
                    ok,
                ),
                setting(
                    "memory.gpu_reserve_bytes",
                    "VRAM reserve",
                    bytes_with_raw(input.config.memory.gpu_reserve_bytes),
                    READONLY,
                    ok,
                ),
                setting(
                    "memory.ram_reserve_bytes",
                    "RAM reserve",
                    bytes_with_raw(input.config.memory.ram_reserve_bytes),
                    READONLY,
                    ok,
                ),
                setting(
                    "memory.nvme_reserve_bytes",
                    "NVMe reserve",
                    bytes_with_raw(input.config.memory.nvme_reserve_bytes),
                    READONLY,
                    ok,
                ),
                setting(
                    "memory.max_prefetch_bytes",
                    "Prefetch cap",
                    bytes_with_raw(input.config.memory.max_prefetch_bytes),
                    READONLY,
                    ok,
                ),
                setting(
                    "memory.allow_engine_managed_paging",
                    "Engine-managed paging",
                    input.config.memory.allow_engine_managed_paging.to_string(),
                    "Off unless a runtime proves safe tensor lifetimes (ADR-005)",
                    ok,
                ),
            ],
        },
        pb::UiSettingsGroup {
            title: "GPU".into(),
            rows: vec![
                setting(
                    "gpu.strategy",
                    "Strategy",
                    input.config.gpu.strategy.clone(),
                    READONLY,
                    ok,
                ),
                setting(
                    "gpu.allow_multi_gpu",
                    "Allow multi-GPU",
                    input.config.gpu.allow_multi_gpu.to_string(),
                    READONLY,
                    ok,
                ),
                setting(
                    "gpu.allow_cross_vendor_model_split",
                    "Cross-vendor split",
                    input.config.gpu.allow_cross_vendor_model_split.to_string(),
                    "Never planned without an explicit opt-in",
                    ok,
                ),
                setting(
                    "gpu.allow_tensor_parallel",
                    "Tensor parallel",
                    input.config.gpu.allow_tensor_parallel.as_str().to_owned(),
                    "Only when the runtime declares support (ADR-004)",
                    ok,
                ),
            ],
        },
        pb::UiSettingsGroup {
            title: "Runtime".into(),
            rows: vec![
                setting(
                    "runtime.auto_install",
                    "Auto-install runtimes",
                    input.config.runtime.auto_install.to_string(),
                    READONLY,
                    ok,
                ),
                setting(
                    "runtime.isolate_workers",
                    "Isolate workers",
                    input.config.runtime.isolate_workers.to_string(),
                    "Workers always run as separate processes (ADR-007)",
                    ok,
                ),
                setting(
                    "runtime.auto_update",
                    "Auto-update runtimes",
                    input.config.runtime.auto_update.to_string(),
                    READONLY,
                    ok,
                ),
            ],
        },
        pb::UiSettingsGroup {
            title: "API".into(),
            rows: vec![
                setting(
                    "api.bind",
                    "Bind address",
                    input.config.api.bind.clone(),
                    READONLY,
                    ok,
                ),
                setting(
                    "api.authentication",
                    "Authentication",
                    input.config.api.authentication.as_str().to_owned(),
                    "A non-loopback bind requires token authentication",
                    ok,
                ),
            ],
        },
    ];

    // Surface each validation finding on the row it belongs to, so a shell
    // highlights the field instead of showing a detached error list.
    for issue in issues {
        for group in &mut groups {
            for row in &mut group.rows {
                if row.key == issue.field {
                    row.severity = severity_for_issue(issue.severity) as i32;
                    row.note = format!("{} — {}", issue.message, row.note);
                }
            }
        }
    }

    pb::UiSettings {
        groups,
        issues: issues
            .iter()
            .map(|issue| pb::UiIssue {
                field: issue.field.clone(),
                severity: severity_for_issue(issue.severity) as i32,
                message: issue.message.clone(),
            })
            .collect(),
        config_file: input.config_file.clone(),
        config_file_exists: input.config_file_exists,
    }
}

fn bytes_with_raw(bytes: u64) -> String {
    format!("{} ({bytes} bytes)", format_bytes(bytes))
}

/// Build the whole snapshot.
pub fn snapshot(
    input: &SnapshotInput,
    issues: &[Issue],
    events: &[pb::UiEventLine],
) -> pb::UiSnapshot {
    pb::UiSnapshot {
        snapshot_version: SNAPSHOT_VERSION,
        generated_at_local: input.generated_at_local.clone(),
        overview: Some(overview(input, events)),
        settings: Some(settings(input, issues)),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use gguf_engine_event_bus::{BusConfig, EventBus};

    fn input(state: CoreState) -> SnapshotInput {
        SnapshotInput {
            state,
            uptime_ms: 3_725_000,
            pid: 4242,
            core_version: "0.1.0".into(),
            endpoint: "local-pipe-test".into(),
            data_root: "C:/data".into(),
            schema_version: 1,
            sqlite_version: "3.50.2".into(),
            bus: BusStats {
                published: 12,
                dropped: 0,
                subscribers: 1,
                capacity: 1024,
                last_seq: 12,
            },
            config: Config::default(),
            config_file: "C:/data/config.toml".into(),
            config_file_exists: false,
            generated_at_local: "2026-01-01 00:00:00 UTC".into(),
        }
    }

    #[test]
    fn uptime_is_formatted_for_a_status_line() {
        assert_eq!(format_uptime(0), "00:00:00");
        assert_eq!(format_uptime(65_000), "00:01:05");
        assert_eq!(format_uptime(3_725_000), "01:02:05");
        assert_eq!(format_uptime(90_061_000), "1d 01:01:01");
    }

    #[test]
    fn bytes_use_binary_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(2_147_483_648), "2.0 GiB");
    }

    #[test]
    fn ready_core_shows_ok_state_and_lists_missing_features() {
        let snapshot = snapshot(&input(CoreState::Ready), &[], &[]);
        let overview = snapshot.overview.expect("overview");
        let state = overview
            .tiles
            .iter()
            .find(|t| t.id == "core.state")
            .unwrap();
        assert_eq!(state.value, "ready");
        assert_eq!(state.severity, pb::UiSeverity::Ok as i32);

        let missing = overview
            .tiles
            .iter()
            .find(|t| t.id == "features.missing")
            .unwrap();
        assert!(missing.value.contains("capabilities"), "{}", missing.value);
        // llama.cpp became real in Task 6 and must no longer be listed here.
        assert!(
            !missing.detail.contains("llama.cpp"),
            "Task 6 capability must not be reported as missing: {}",
            missing.detail
        );
        // Hardware became real in Task 4 and must no longer be listed here.
        assert!(
            !missing.detail.contains("Hardware inventory"),
            "implemented capabilities must not be reported as missing: {}",
            missing.detail
        );
        assert!(
            !missing.detail.contains("Model registry")
                && !missing.detail.contains("Model download"),
            "Task 5 capabilities must not be reported as missing: {}",
            missing.detail
        );

        // Task 7 capabilities must no longer be listed as missing either.
        assert!(
            !missing.detail.contains("Multi-GPU") && !missing.detail.contains("Memory paging"),
            "Task 7 capabilities must not be reported as missing: {}",
            missing.detail
        );

        // A ready core with unimplemented capabilities still asks for
        // attention: the user cannot use those features yet.
        assert!(overview.attention_required);
    }

    #[test]
    fn stopped_core_is_an_error_tile() {
        let overview = snapshot(&input(CoreState::Stopped), &[], &[])
            .overview
            .unwrap();
        let state = overview
            .tiles
            .iter()
            .find(|t| t.id == "core.state")
            .unwrap();
        assert_eq!(state.severity, pb::UiSeverity::Error as i32);
        assert!(overview.attention_required);
    }

    #[test]
    fn every_feature_row_is_labelled_and_explained() {
        let overview = snapshot(&input(CoreState::Ready), &[], &[])
            .overview
            .unwrap();
        assert_eq!(overview.features.len(), features().len());
        for row in &overview.features {
            assert!(!row.label.is_empty(), "{} has no label", row.id);
            assert!(!row.status_text.is_empty(), "{} has no status text", row.id);
            if row.status != pb::CapabilityStatus::Supported as i32 {
                assert!(!row.reason.is_empty(), "{} has no reason", row.id);
            }
        }
    }

    #[test]
    fn settings_are_read_only_with_a_reason() {
        let settings = snapshot(&input(CoreState::Ready), &[], &[])
            .settings
            .unwrap();
        assert!(settings.groups.len() >= 5);
        for group in &settings.groups {
            assert!(!group.title.is_empty());
            for row in &group.rows {
                assert!(!row.key.is_empty());
                assert!(!row.editable, "{} must not claim to be editable", row.key);
                assert!(!row.note.is_empty(), "{} has no note", row.key);
            }
        }
    }

    #[test]
    fn configuration_issues_mark_the_offending_row() {
        let issues = vec![Issue {
            field: "core.api_port".into(),
            severity: ConfigSeverity::Error,
            message: "port 0 is not a valid listen port".into(),
        }];
        let settings = snapshot(&input(CoreState::Ready), &issues, &[])
            .settings
            .unwrap();
        let row = settings
            .groups
            .iter()
            .flat_map(|g| &g.rows)
            .find(|r| r.key == "core.api_port")
            .expect("row present");
        assert_eq!(row.severity, pb::UiSeverity::Error as i32);
        assert!(row.note.contains("not a valid listen port"));
        assert_eq!(settings.issues.len(), 1);
    }

    #[test]
    fn health_ticks_are_not_shown_but_state_changes_are() {
        let bus = EventBus::new(BusConfig::default());
        bus.publish(EventPayload::HealthTick {
            uptime_ms: 1,
            subscribers: 0,
        })
        .unwrap();
        bus.publish(EventPayload::CoreStateChanged {
            from: CoreState::Starting,
            to: CoreState::Ready,
        })
        .unwrap();
        bus.publish(EventPayload::Notice {
            level: NoticeLevel::Warning,
            message: "event backlog".into(),
        })
        .unwrap();

        let lines: Vec<pb::UiEventLine> = bus
            .replay_after(0)
            .iter()
            .filter_map(|event| event_line(event.seq, event.at_unix_ms, &event.payload))
            .collect();
        assert_eq!(lines.len(), 2, "health ticks are noise on Overview");
        assert_eq!(lines[0].severity, pb::UiSeverity::Ok as i32);
        assert!(lines[0].text.contains("starting -> ready"));
        assert_eq!(lines[1].severity, pb::UiSeverity::Warning as i32);
        assert!(!lines[0].at_local.is_empty());
    }

    #[test]
    fn dropped_events_raise_a_warning_tile() {
        let mut input = input(CoreState::Ready);
        input.bus.dropped = 3;
        let overview = snapshot(&input, &[], &[]).overview.unwrap();
        let tile = overview
            .tiles
            .iter()
            .find(|t| t.id == "events.bus")
            .unwrap();
        assert_eq!(tile.severity, pb::UiSeverity::Warning as i32);
        assert!(tile.detail.contains("3 dropped"));
    }

    #[test]
    fn snapshot_survives_a_wire_round_trip() {
        let snapshot = snapshot(&input(CoreState::Degraded), &[], &[]);
        let bytes = prost::Message::encode_to_vec(&snapshot);
        let decoded = <pb::UiSnapshot as prost::Message>::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.snapshot_version, SNAPSHOT_VERSION);
        assert_eq!(
            decoded.overview.unwrap().tiles.len(),
            snapshot.overview.unwrap().tiles.len()
        );
    }
}
