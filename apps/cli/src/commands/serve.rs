//! `gguf serve` — run the core daemon in the foreground.
//!
//! Task 1 delivers the process skeleton: directories, configuration, the
//! single-instance lock, the lifecycle state machine and the event bus, plus a
//! heartbeat task that emits health ticks. IPC (gRPC over a named pipe / Unix
//! socket), the HTTP API and the database arrive in Task 2; until then the
//! daemon honestly reports those components as unavailable.

use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gguf_engine_core::{ComponentHealth, HealthMonitor, InstanceLock, Lifecycle};
use gguf_engine_event_bus::{BusConfig, CoreState, EventBus, EventPayload};
use serde::Serialize;

use crate::commands::Context;
use crate::output;

#[derive(Debug, Serialize)]
struct ServeSummary {
    ok: bool,
    data_root: String,
    lock_file: String,
    states: Vec<String>,
    ticks: u64,
}

/// Run until Ctrl+C (or SIGTERM on Unix).
pub async fn serve(ctx: &Context, json: bool) -> Result<ExitCode> {
    ctx.paths
        .ensure_dirs()
        .context("creating data directories")?;

    let lock = InstanceLock::acquire(ctx.paths.lock_file())
        .context("acquiring the single instance lock")?;
    tracing::info!(
        data_root = %ctx.paths.root().display(),
        lock = %lock.path().display(),
        pid = std::process::id(),
        "starting GGUF AI Engine core"
    );

    let bus = EventBus::new(BusConfig {
        capacity: 1024,
        replay_limit: 256,
        label: "core".to_owned(),
    });
    let lifecycle = Lifecycle::new(bus.clone());
    let monitor = HealthMonitor::new();

    let boot = lifecycle.boot().context("booting the core lifecycle")?;
    let mut states: Vec<String> = boot.iter().map(|c| c.to.as_str().to_owned()).collect();

    let components = vec![
        ComponentHealth::healthy("lifecycle"),
        ComponentHealth::healthy("event-bus"),
        ComponentHealth::unhealthy("ipc", "planned for Task 2: no listener yet"),
        ComponentHealth::unhealthy("database", "planned for Task 2: no schema yet"),
    ];
    let report = monitor.report(lifecycle.current(), bus.stats(), components);
    // A core that cannot yet serve its API is Degraded, not Ready: the UIs
    // must be able to show the truth instead of a fake "connected" state.
    lifecycle
        .transition(CoreState::Degraded)
        .context("marking the core degraded")?;
    states.push(CoreState::Degraded.as_str().to_owned());
    tracing::warn!(
        state = lifecycle.current().as_str(),
        "core is running but IPC and database are not implemented yet"
    );

    if !json {
        println!(
            "core       : {} {}",
            gguf_engine_core::CORE_NAME,
            gguf_engine_core::CORE_VERSION
        );
        println!("data root  : {}", ctx.paths.root().display());
        println!("lock       : {}", lock.path().display());
        println!("health     : {}", report.summary());
        println!(
            "state      : {} (IPC and database are planned for Task 2)",
            lifecycle.current().as_str()
        );
        println!("press Ctrl+C to stop");
    }

    let interval = Duration::from_millis(ctx.config.core.health_interval_ms.max(100));
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut subscriber = bus.subscribe();
    let mut ticks: u64 = 0;

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("shutdown signal received");
                break;
            }
            _ = ticker.tick() => {
                ticks += 1;
                let uptime_ms = monitor.uptime_ms();
                if let Err(err) = bus.publish(EventPayload::HealthTick {
                    uptime_ms,
                    subscribers: bus.subscriber_count(),
                }) {
                    tracing::warn!(error = %err, "failed to publish health tick");
                }
                tracing::debug!(uptime_ms, ticks, "health tick");
            }
            event = subscriber.recv() => {
                match event {
                    Some(event) => tracing::trace!(seq = event.seq, "event"),
                    None => break,
                }
            }
        }
    }

    lifecycle.shutdown().context("shutting down the core")?;
    states.push(CoreState::Stopped.as_str().to_owned());

    let summary = ServeSummary {
        ok: true,
        data_root: ctx.paths.root().display().to_string(),
        lock_file: lock.path().display().to_string(),
        states,
        ticks,
    };
    let stats = bus.stats();
    output::emit(json, &summary, |summary| {
        println!("stopped    : {} health tick(s) emitted", summary.ticks);
    });
    tracing::info!(
        published = stats.published,
        dropped = stats.dropped,
        "core stopped"
    );
    drop(lock);
    Ok(ExitCode::SUCCESS)
}
