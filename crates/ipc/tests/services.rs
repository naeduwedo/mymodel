//! Service-level tests over a loopback transport.
//!
//! These exercise the real gRPC stack (codec, status mapping, streaming) without
//! depending on a platform socket, so they run identically on Linux, macOS and
//! Windows CI. The platform transports (named pipe, Unix socket) are covered by
//! the end-to-end tests in `tests/`.

use std::sync::Arc;
use std::time::Duration;

use gguf_engine_event_bus::{BusConfig, CoreState as LifecycleState, EventBus, EventPayload};
use gguf_engine_ipc::state::CoreState;
use gguf_engine_ipc::{LifecycleHandle, serve_loopback};
use gguf_engine_protocol::pb;
use tokio_stream::StreamExt;

fn state() -> CoreState {
    CoreState {
        bus: EventBus::new(BusConfig {
            capacity: 64,
            replay_limit: 32,
            label: "test".into(),
        }),
        lifecycle: Arc::new(LifecycleHandle::new(LifecycleState::Degraded)),
        database: gguf_engine_database::Database::open_in_memory().unwrap(),
        started_at_unix_ms: 1_700_000_000_000,
        instance_id: "test-instance".into(),
        protocol_version: gguf_engine_protocol::PROTOCOL_VERSION,
        features: gguf_engine_protocol::features(),
        config: gguf_engine_config::Config::default(),
        config_issues: Vec::new(),
        config_file: "config.toml".into(),
        config_file_exists: false,
        data_root: "data".into(),
        endpoint: "test".into(),
        hardware: std::sync::Arc::new(gguf_engine_ipc::services::HardwareCache::default()),
    }
}

#[tokio::test]
async fn health_reports_through_a_real_channel() {
    let state = state();
    let server = serve_loopback(state).await.unwrap();
    let client = gguf_engine_ipc::connect_to(server.endpoint())
        .await
        .unwrap();

    let health = client.health().await.unwrap();
    assert_eq!(health.state, pb::CoreState::Degraded as i32);
    assert_eq!(health.pid, std::process::id());
    assert!(health.components.iter().any(|c| c.name == "database"));
    assert!(health.bus.is_some());

    client.ping().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn events_replay_then_stream_live() {
    let state = state();
    // Publish before any client connects: the replay ring must deliver them.
    for i in 0..3 {
        state
            .bus
            .publish(EventPayload::Notice {
                level: gguf_engine_event_bus::NoticeLevel::Info,
                message: format!("before-{i}"),
            })
            .unwrap();
    }

    let server = serve_loopback(state.clone()).await.unwrap();
    let client = gguf_engine_ipc::connect_to(server.endpoint())
        .await
        .unwrap();
    let mut stream = Box::pin(client.subscribe(0).await.unwrap());

    for i in 0..3 {
        let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("replayed event")
            .expect("stream open")
            .unwrap();
        assert_eq!(event.seq, i + 1);
        assert!(event.payload_json.contains(&format!("before-{i}")));
    }

    // A live event published after subscribing must follow.
    state
        .bus
        .publish(EventPayload::Notice {
            level: gguf_engine_event_bus::NoticeLevel::Warning,
            message: "live".into(),
        })
        .unwrap();
    let live = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("live event")
        .unwrap()
        .unwrap();
    assert_eq!(live.seq, 4);
    assert!(live.payload_json.contains("live"));

    drop(stream);
    drop(client);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn subscribe_from_a_sequence_number_skips_older_events() {
    let state = state();
    for i in 0..5 {
        state
            .bus
            .publish(EventPayload::HealthTick {
                uptime_ms: i,
                subscribers: 0,
            })
            .unwrap();
    }
    let server = serve_loopback(state).await.unwrap();
    let client = gguf_engine_ipc::connect_to(server.endpoint())
        .await
        .unwrap();
    let mut stream = Box::pin(client.subscribe(3).await.unwrap());

    let first = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first.seq, 4, "replay must start after the requested seq");

    drop(stream);
    drop(client);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn connecting_to_a_dead_endpoint_is_reported_as_not_running() {
    let tmp = tempfile::tempdir().unwrap();
    let err = gguf_engine_ipc::connect(tmp.path()).await.unwrap_err();
    assert!(
        matches!(err, gguf_engine_ipc::IpcError::NotRunning(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn unimplemented_services_are_not_registered() {
    let state = state();
    let server = serve_loopback(state).await.unwrap();
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{}", server.endpoint()))
        .unwrap()
        .connect()
        .await
        .unwrap();

    // Hardware became real in Task 4: it answers with real probe results, and
    // must never claim support while reporting nothing.
    let mut hardware = pb::hardware_service_client::HardwareServiceClient::new(channel.clone());
    let report = hardware
        .get_hardware(pb::HardwareRequest {})
        .await
        .expect("hardware is implemented since Task 4")
        .into_inner();
    assert!(report.cpu.is_some());
    assert!(report.memory.is_some());
    if report.gpus.is_empty() {
        assert_ne!(report.status, pb::CapabilityStatus::Supported as i32);
        assert!(!report.unavailable_reason.is_empty());
    }

    let mut inference = pb::inference_service_client::InferenceServiceClient::new(channel);
    let err = inference
        .chat(pb::ChatRequest {
            model_id: "x".into(),
            messages: vec![],
            task_id: None,
            max_tokens: None,
            temperature: None,
            stream: None,
        })
        .await
        .expect_err("inference must not be served before Task 6");
    assert_eq!(err.code(), tonic::Code::Unimplemented);

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_closes_the_endpoint() {
    let state = state();
    let server = serve_loopback(state).await.unwrap();
    let endpoint = server.endpoint().to_owned();
    gguf_engine_ipc::connect_to(&endpoint).await.unwrap();
    server.shutdown().await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        gguf_engine_ipc::connect_to(&endpoint).await.is_err(),
        "a stopped core must not accept connections"
    );
}
