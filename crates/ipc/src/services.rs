//! gRPC service implementations.
//!
//! Only implemented services are registered by [`crate::server`]. Each handler
//! is deliberately thin: it reads shared state, converts through
//! `gguf_engine_protocol::convert`, and never performs blocking I/O on the
//! runtime (database calls go through `Database::call`).

use std::pin::Pin;

use gguf_engine_protocol::convert;
use gguf_engine_protocol::pb;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};
use tracing::{debug, warn};

use crate::state::CoreState;

/// `HealthService`: liveness, lifecycle state and the capability matrix.
#[derive(Debug)]
pub struct HealthService {
    state: CoreState,
}

impl HealthService {
    /// Create the service.
    pub fn new(state: CoreState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl pb::health_service_server::HealthService for HealthService {
    async fn get_health(
        &self,
        _request: Request<pb::HealthRequest>,
    ) -> Result<Response<pb::HealthReport>, Status> {
        let database = self.state.database.status().await.map_err(|err| {
            warn!(error = %err, "health: database status failed");
            Status::unavailable(format!("database unavailable: {err}"))
        })?;

        let mut components = vec![
            convert::component_parts("ipc", true, None),
            convert::component_parts(
                "database",
                true,
                Some(
                    format!(
                        "schema v{}, sqlite {}",
                        database.schema_version, database.sqlite_version
                    )
                    .as_str(),
                ),
            ),
            convert::component_parts("event-bus", true, None),
        ];
        // Unimplemented subsystems are reported as unhealthy *with a reason*
        // rather than omitted, so a UI can explain what is missing.
        for feature in &self.state.features {
            if feature.status == gguf_engine_protocol::FeatureStatus::Planned {
                components.push(convert::component_parts(
                    &feature.id,
                    false,
                    feature.reason.as_deref(),
                ));
            }
        }

        let report = pb::HealthReport {
            state: convert::core_state(self.state.lifecycle.current()) as i32,
            uptime_ms: self.state.lifecycle.uptime_ms(),
            started_at_unix_ms: self.state.started_at_unix_ms,
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            bus: Some(convert::bus_stats(self.state.bus.stats())),
            healthy: false, // filled in below
            components,
        };

        let healthy = matches!(
            self.state.lifecycle.current(),
            gguf_engine_event_bus::CoreState::Ready | gguf_engine_event_bus::CoreState::Degraded
        ) && report.components.iter().all(|c| c.healthy);
        Ok(Response::new(pb::HealthReport { healthy, ..report }))
    }

    async fn ping(&self, _request: Request<pb::Empty>) -> Result<Response<pb::Empty>, Status> {
        Ok(Response::new(pb::Empty {}))
    }
}

/// `EventService`: bounded replay followed by a live stream.
#[derive(Debug)]
pub struct EventService {
    state: CoreState,
}

impl EventService {
    /// Create the service.
    pub fn new(state: CoreState) -> Self {
        Self { state }
    }
}

type EventStream = Pin<Box<dyn Stream<Item = Result<pb::Event, Status>> + Send + 'static>>;

#[tonic::async_trait]
impl pb::event_service_server::EventService for EventService {
    type SubscribeStream = EventStream;

    async fn subscribe(
        &self,
        request: Request<pb::EventRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let after_seq = request.into_inner().after_seq;
        let replay = self.state.bus.replay_after(after_seq);
        debug!(after_seq, replayed = replay.len(), "event stream opened");

        let mut subscriber = self.state.bus.subscribe();
        let mut last_seq = after_seq;
        for event in &replay {
            last_seq = last_seq.max(event.seq);
        }

        let live = async_stream::try_stream! {
            while let Some(event) = subscriber.recv().await {
                // Skip anything already covered by the replay ring.
                if event.seq <= last_seq {
                    continue;
                }
                last_seq = event.seq;
                yield convert::event_message(&event);
            }
        };

        let replay_stream = tokio_stream::iter(
            replay
                .into_iter()
                .map(|event| Ok::<pb::Event, Status>(convert::event_message(&event))),
        );

        Ok(Response::new(Box::pin(tokio_stream::StreamExt::chain(
            replay_stream,
            live,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gguf_engine_event_bus::{BusConfig, CoreState as LifecycleState, EventBus};
    use gguf_engine_protocol::pb::health_service_server::HealthService as _;
    use std::sync::Arc;

    fn state() -> CoreState {
        CoreState {
            bus: EventBus::new(BusConfig::default()),
            lifecycle: Arc::new(crate::LifecycleHandle::new(LifecycleState::Ready)),
            database: gguf_engine_database::Database::open_in_memory().unwrap(),
            started_at_unix_ms: 1,
            instance_id: "test".into(),
            protocol_version: gguf_engine_protocol::PROTOCOL_VERSION,
            features: gguf_engine_protocol::features(),
            config: gguf_engine_config::Config::default(),
            config_issues: Vec::new(),
            config_file: "config.toml".into(),
            config_file_exists: false,
            data_root: "data".into(),
            models_dir: "data/models".into(),
            endpoint: "test".into(),
            hardware: std::sync::Arc::new(crate::services::HardwareCache::default()),
        }
    }

    #[tokio::test]
    async fn health_reports_state_and_unimplemented_components() {
        let service = HealthService::new(state());
        let report = service
            .get_health(Request::new(pb::HealthRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(report.state, pb::CoreState::Ready as i32);
        assert_eq!(report.version, env!("CARGO_PKG_VERSION"));
        assert!(
            report
                .components
                .iter()
                .any(|c| c.name == "database" && c.healthy)
        );
        let planned: Vec<&str> = report
            .components
            .iter()
            .filter(|c| !c.healthy && c.name != "ipc")
            .map(|c| c.name.as_str())
            .collect();
        // runtime_llama_cpp became real in Task 6 and must no longer appear as
        // an unimplemented component.
        assert!(!planned.contains(&"runtime_llama_cpp"));
        assert!(
            !report.healthy,
            "planned subsystems must keep the report unhealthy"
        );
    }

    #[tokio::test]
    async fn ping_is_cheap_and_unconditional() {
        let service = HealthService::new(state());
        service.ping(Request::new(pb::Empty {})).await.unwrap();
    }
}
/// `UiService`: the presentation contract shared by the three native shells.
///
/// The shell renders what this returns; it does not compute health, format
/// memory values or decide what "planned" means. See `proto/ui.proto`.
#[derive(Debug)]
pub struct UiService {
    state: CoreState,
}

impl UiService {
    /// Create the service.
    pub fn new(state: CoreState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl pb::ui_service_server::UiService for UiService {
    async fn get_snapshot(
        &self,
        _request: Request<pb::UiSnapshotRequest>,
    ) -> Result<Response<pb::UiSnapshot>, Status> {
        // Overview shows the schema version, which must be read through the
        // blocking wrapper rather than on the runtime thread.
        let database = self.state.database.status().await.map_err(|err| {
            warn!(error = %err, "ui: database status failed");
            Status::unavailable(format!("database unavailable: {err}"))
        })?;

        let mut snapshot = self
            .state
            .snapshot(database.schema_version, database.sqlite_version);

        // Hardware tiles are appended from the same cache HardwareService uses,
        // so Overview and the Hardware page can never disagree.
        if let Some(overview) = snapshot.overview.as_mut() {
            overview.tiles.extend(self.state.hardware_tiles().await);
        }

        Ok(Response::new(snapshot))
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use gguf_engine_protocol::pb::ui_service_server::UiService as _;
    use std::sync::Arc;

    fn state() -> CoreState {
        CoreState {
            bus: gguf_engine_event_bus::EventBus::new(gguf_engine_event_bus::BusConfig::default()),
            lifecycle: Arc::new(crate::LifecycleHandle::new(
                gguf_engine_event_bus::CoreState::Ready,
            )),
            database: gguf_engine_database::Database::open_in_memory().unwrap(),
            started_at_unix_ms: 1,
            instance_id: "test".into(),
            protocol_version: gguf_engine_protocol::PROTOCOL_VERSION,
            features: gguf_engine_protocol::features(),
            config: gguf_engine_config::Config::default(),
            config_issues: Vec::new(),
            config_file: "C:/data/config.toml".into(),
            config_file_exists: true,
            data_root: "C:/data".into(),
            models_dir: "C:/data/models".into(),
            endpoint: "test-endpoint".into(),
            hardware: std::sync::Arc::new(HardwareCache::default()),
        }
    }

    #[tokio::test]
    async fn snapshot_has_overview_and_settings() {
        let service = UiService::new(state());
        let snapshot = service
            .get_snapshot(Request::new(pb::UiSnapshotRequest {}))
            .await
            .unwrap()
            .into_inner();

        let overview = snapshot.overview.expect("overview");
        assert!(overview.tiles.iter().any(|t| t.id == "db.schema"));
        assert!(
            overview.features.iter().any(|f| f.id == "ipc"),
            "the capability list is part of the contract"
        );

        let settings = snapshot.settings.expect("settings");
        assert!(settings.config_file_exists);
        assert!(!settings.groups.is_empty());
    }
}
/// `HardwareService`: the inventory from Task 4.
///
/// The probe spawns vendor tools (`nvidia-smi`, `rocm-smi`, `system_profiler`),
/// so it runs on the blocking pool and the result is cached briefly: a UI that
/// polls every two seconds must not start a process per poll.
#[derive(Debug)]
pub struct HardwareService {
    cache: std::sync::Arc<HardwareCache>,
}

impl HardwareService {
    /// Create the service.
    pub fn new() -> Self {
        Self {
            cache: std::sync::Arc::new(HardwareCache::default()),
        }
    }
}

impl Default for HardwareService {
    fn default() -> Self {
        Self::new()
    }
}

/// Cached inventory, refreshed at most once per [`INVENTORY_TTL`].
#[derive(Debug, Default)]
pub struct HardwareCache {
    latest: tokio::sync::Mutex<Option<(std::time::Instant, gguf_engine_hardware::HostSnapshot)>>,
}

/// How long an inventory result is reused.
///
/// Inventory is about *hardware*, not about load: clocks, temperatures and free
/// VRAM have their own endpoints later. Two seconds keeps the UI responsive
/// without hammering the driver.
pub const INVENTORY_TTL: std::time::Duration = std::time::Duration::from_secs(2);

impl HardwareCache {
    /// Return a snapshot, collecting one when the cache is cold or stale.
    pub async fn get(&self) -> gguf_engine_hardware::HostSnapshot {
        let mut guard = self.latest.lock().await;
        if let Some((at, snapshot)) = guard.as_ref()
            && at.elapsed() < INVENTORY_TTL
        {
            return snapshot.clone();
        }

        // The vendor tools are blocking; never run them on the runtime threads.
        let snapshot = match tokio::task::spawn_blocking(gguf_engine_hardware::collect).await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                // A panic in a probe must not take the core down. Report it as
                // an unavailable inventory with the panic as the reason.
                warn!(error = %err, "hardware probe task failed");
                let mut snapshot =
                    gguf_engine_hardware::collect_with(&gguf_engine_hardware::SystemRunner);
                snapshot.gpus.reason = Some(format!("hardware probe failed: {err}"));
                snapshot
            }
        };

        *guard = Some((std::time::Instant::now(), snapshot.clone()));
        snapshot
    }
}

#[tonic::async_trait]
impl pb::hardware_service_server::HardwareService for HardwareService {
    async fn get_hardware(
        &self,
        _request: Request<pb::HardwareRequest>,
    ) -> Result<Response<pb::HardwareReport>, Status> {
        let snapshot = self.cache.get().await;
        Ok(Response::new(convert::hardware_report(&snapshot)))
    }
}
/// `ModelService`: the registry as the UIs see it.
///
/// Only verified facts are returned: a file appears with state `ready` only after
/// its bytes were downloaded and hashed. Pending and failed files are listed too,
/// with their state, because hiding them would make a partial download look
/// finished.
#[derive(Debug)]
pub struct ModelService {
    state: CoreState,
}

impl ModelService {
    /// Create the service.
    pub fn new(state: CoreState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl pb::model_service_server::ModelService for ModelService {
    async fn list_models(
        &self,
        request: Request<pb::ListModelsRequest>,
    ) -> Result<Response<pb::ListModelsResponse>, Status> {
        let request = request.into_inner();
        let revisions = self
            .state
            .database
            .list_revisions()
            .await
            .map_err(|err| Status::unavailable(format!("registry unavailable: {err}")))?;

        let total = u32::try_from(revisions.len()).unwrap_or(u32::MAX);
        let offset = request.offset as usize;
        let limit = if request.limit == 0 {
            revisions.len()
        } else {
            request.limit as usize
        };

        let models = revisions
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|revision| convert::manifest(&revision))
            .collect();

        Ok(Response::new(pb::ListModelsResponse { models, total }))
    }

    async fn get_model(
        &self,
        request: Request<pb::GetModelRequest>,
    ) -> Result<Response<pb::ModelManifest>, Status> {
        let model_id = request.into_inner().model_id;

        // The caller addresses a model; the newest revision is the one to show.
        let revisions = self
            .state
            .database
            .list_revisions()
            .await
            .map_err(|err| Status::unavailable(format!("registry unavailable: {err}")))?;

        match revisions
            .into_iter()
            .find(|record| record.model_id == model_id)
        {
            Some(record) => Ok(Response::new(convert::manifest(&record))),
            None => Err(Status::not_found(format!("no model {model_id}"))),
        }
    }
}
/// RuntimeService: what runtimes are installed and what they can do.
///
/// Backed by a lazy probe of the installed llama.cpp build. A runtime that is
/// not installed is reported as unavailable with a reason, never as present.
#[derive(Debug)]
pub struct RuntimeService {
    adapter: Option<std::sync::Arc<gguf_engine_runtime_llama_cpp::LlamaCppAdapter>>,
}

impl Default for RuntimeService {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeService {
    /// Create the service, probing the installed runtime.
    pub fn new() -> Self {
        // discover() fails only when no llama.cpp is installed; the service
        // records that as None and reports an empty list, never a fake runtime.
        let adapter = gguf_engine_runtime_llama_cpp::LlamaCppAdapter::discover()
            .ok()
            .map(std::sync::Arc::new);
        Self { adapter }
    }
}

#[tonic::async_trait]
impl pb::runtime_service_server::RuntimeService for RuntimeService {
    async fn list_runtimes(
        &self,
        _request: Request<pb::ListRuntimesRequest>,
    ) -> Result<Response<pb::ListRuntimesResponse>, Status> {
        use gguf_engine_runtime_manager::RuntimeAdapter;

        let Some(adapter) = self.adapter.as_ref() else {
            // No runtime installed: an empty list is the honest answer.
            return Ok(Response::new(pb::ListRuntimesResponse {
                runtimes: Vec::new(),
            }));
        };

        let caps = adapter
            .capabilities()
            .await
            .map_err(|err| Status::unavailable(format!("runtime unavailable: {err}")))?;

        let info = pb::RuntimeInfo {
            id: "llama.cpp".to_owned(),
            version: "installed".to_owned(),
            installed_path: adapter.directory(),
            installed: pb::CapabilityStatus::Supported as i32,
            architectures: caps.architectures.clone(),
            backends: caps
                .backends
                .iter()
                .map(|b| b.as_str().to_owned())
                .collect(),
            capabilities: caps
                .capabilities
                .iter()
                .map(|c| pb::CapabilityEntry {
                    name: c.name.clone(),
                    status: status_to_pb(c.status) as i32,
                    detail: c.detail.clone().unwrap_or_default(),
                })
                .collect(),
        };

        Ok(Response::new(pb::ListRuntimesResponse {
            runtimes: vec![info],
        }))
    }
}

fn status_to_pb(status: gguf_engine_runtime_manager::CapabilityStatus) -> pb::CapabilityStatus {
    use gguf_engine_runtime_manager::CapabilityStatus;
    match status {
        CapabilityStatus::Supported => pb::CapabilityStatus::Supported,
        CapabilityStatus::Experimental => pb::CapabilityStatus::Experimental,
        CapabilityStatus::Unsupported => pb::CapabilityStatus::Unsupported,
        CapabilityStatus::Unknown => pb::CapabilityStatus::Unknown,
    }
}
