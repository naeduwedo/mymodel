//! Core-level integration tests: configuration, paths, event bus and the
//! lifecycle are exercised together the way the daemon wires them.

use std::path::Path;

use gguf_engine_config::{AppPaths, Config, Origin};
use gguf_engine_core::{ComponentHealth, HealthMonitor, InstanceLock, Lifecycle};
use gguf_engine_event_bus::{BusConfig, CoreState, EventBus, EventPayload};

fn bus() -> EventBus {
    EventBus::new(BusConfig {
        capacity: 256,
        replay_limit: 64,
        label: "test".to_owned(),
    })
}

#[test]
fn full_startup_path_from_config_to_ready() {
    let tmp = tempfile::tempdir().unwrap();

    // 1. configuration
    let config = Config::default();
    gguf_engine_config::validate(&config).expect("defaults are valid");

    // 2. directories
    let paths = AppPaths::from_config_with_origin(tmp.path(), Origin::Explicit, &config).unwrap();
    paths.ensure_dirs().unwrap();
    assert!(paths.models().is_dir());
    assert!(paths.lock_file().parent().unwrap().is_dir());

    // 3. single-instance lock
    let lock = InstanceLock::acquire(paths.lock_file()).unwrap();

    // 4. lifecycle + health
    let bus = bus();
    let lifecycle = Lifecycle::new(bus.clone());
    let monitor = HealthMonitor::new();
    lifecycle.boot().unwrap();
    let health = monitor.report(
        lifecycle.current(),
        bus.stats(),
        vec![
            ComponentHealth::healthy("lifecycle"),
            ComponentHealth::healthy("event-bus"),
        ],
    );
    assert!(health.is_healthy());

    // 5. the UI can catch up on what happened while it was not connected.
    let replay = bus.replay_after(0);
    assert_eq!(replay.len(), 2);
    assert!(matches!(
        replay[1].payload,
        EventPayload::CoreStateChanged {
            to: CoreState::Ready,
            ..
        }
    ));

    // 6. clean shutdown releases resource-less components in order.
    let changes = lifecycle.shutdown().unwrap();
    assert_eq!(changes.last().unwrap().to, CoreState::Stopped);
    drop(lock);

    // 7. the lock is reusable after a clean stop.
    let _lock = InstanceLock::acquire(paths.lock_file()).unwrap();
}

#[test]
fn degraded_transition_is_visible_to_a_late_subscriber() {
    let bus = bus();
    let lifecycle = Lifecycle::new(bus.clone());
    lifecycle.boot().unwrap();
    lifecycle.transition(CoreState::Degraded).unwrap();

    // A UI attaching now sees the whole history in order.
    let seen: Vec<CoreState> = bus
        .replay_after(0)
        .into_iter()
        .filter_map(|event| match event.payload {
            EventPayload::CoreStateChanged { to, .. } => Some(to),
            _ => None,
        })
        .collect();
    assert_eq!(
        seen,
        vec![CoreState::Starting, CoreState::Ready, CoreState::Degraded]
    );
    assert_eq!(lifecycle.current(), CoreState::Degraded);
}

#[test]
fn configuration_round_trips_through_a_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");

    let mut config = Config::default();
    config.core.api_port = 20001;
    config.api.authentication = gguf_engine_config::AuthenticationMode::Token;
    config.api.bind = "0.0.0.0".into();
    std::fs::write(&path, config.to_toml_string().unwrap()).unwrap();

    let loaded = Config::load_or_default(&path).unwrap();
    assert_eq!(loaded, config);
    gguf_engine_config::validate(&loaded).expect("token auth makes LAN binding valid");

    // Any other bind without a token must be refused.
    let mut bad = loaded.clone();
    bad.api.authentication = gguf_engine_config::AuthenticationMode::LocalOnly;
    assert!(gguf_engine_config::validate(&bad).is_err());
}

#[test]
fn configured_directories_cannot_escape_the_data_root() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.paths.models = "../outside".to_owned();
    let err = AppPaths::from_config(tmp.path(), &config).unwrap_err();
    assert!(err.to_string().contains("escapes"), "{err}");
}

#[test]
fn health_reports_are_serialisable_for_the_ui() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(Path::new(tmp.path()).is_dir());
    let bus = bus();
    let monitor = HealthMonitor::new();
    let report = monitor.report(CoreState::Ready, bus.stats(), vec![]);
    let json = serde_json::to_string(&report).unwrap();
    let decoded: gguf_engine_core::HealthReport = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, report);
}

#[tokio::test]
async fn a_subscriber_added_after_boot_receives_live_events() {
    let bus = bus();
    let lifecycle = std::sync::Arc::new(Lifecycle::new(bus.clone()));
    lifecycle.boot().unwrap();

    let mut subscriber = bus.subscribe();
    let lc = lifecycle.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        lc.transition(CoreState::Degraded).unwrap();
    });

    let event = tokio::time::timeout(std::time::Duration::from_secs(2), subscriber.recv())
        .await
        .expect("event arrived before timeout")
        .expect("subscriber open");
    assert_eq!(
        event.payload,
        EventPayload::CoreStateChanged {
            from: CoreState::Ready,
            to: CoreState::Degraded
        }
    );
}
