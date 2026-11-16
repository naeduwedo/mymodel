//! `gguf-core` — the GGUF AI Engine core daemon.
//!
//! Startup order (each step must succeed before the next one runs, so a partial
//! failure leaves nothing half-open):
//!
//! 1. resolve the data root and load/validate `config.toml`;
//! 2. create the directory tree;
//! 3. take the single-instance lock — a second core would fight over the
//!    database, the socket and the GPU leases;
//! 4. open and migrate the SQLite database;
//! 5. start the IPC listener (named pipe / Unix socket);
//! 6. transition `Stopped -> Starting -> Ready` and publish it to the event bus.
//!
//! Shutdown reverses 5 and 6 and releases the lock last.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use gguf_engine_config::{AppPaths, Config};
use gguf_engine_core::{ComponentHealth, HealthMonitor, InstanceLock, Lifecycle, LockError};
use gguf_engine_database::Database;
use gguf_engine_event_bus::{BusConfig, CoreState, EventBus, EventPayload};
use gguf_engine_ipc::{CoreState as IpcState, LifecycleHandle};
use gguf_engine_telemetry::LogFormat;

/// Exit code used when another core already owns the data directory.
const EXIT_ALREADY_RUNNING: u8 = 3;
/// Exit code used when the IPC endpoint cannot be bound.
const EXIT_IPC_UNAVAILABLE: u8 = 4;

#[derive(Debug, Parser)]
#[command(name = "gguf-core", version, about = "GGUF AI Engine core daemon")]
struct Args {
    /// Override the application data directory.
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Override the configuration file path.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Log format.
    #[arg(long, value_enum, default_value_t = LogFormat::Pretty)]
    log_format: LogFormat,

    /// Print a startup report as JSON and exit without serving.
    #[arg(long)]
    check: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let format = args.log_format;

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("gguf-core: failed to start the async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async_main(args, format))
}

async fn async_main(args: Args, format: LogFormat) -> ExitCode {
    let (root, origin) = match AppPaths::resolve_root(args.data_dir.as_deref()) {
        Ok(resolved) => resolved,
        Err(err) => {
            eprintln!("gguf-core: {err}");
            return ExitCode::FAILURE;
        }
    };
    let config_file = args
        .config
        .clone()
        .unwrap_or_else(|| root.join(gguf_engine_config::CONFIG_FILE_NAME));

    let config = match Config::load_or_default(&config_file) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("gguf-core: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = gguf_engine_config::validate(&config) {
        eprintln!("gguf-core: {err}");
        return ExitCode::FAILURE;
    }

    if let Err(err) = gguf_engine_telemetry::tracing_setup(&config.core.log_level, format) {
        eprintln!("gguf-core: logger already installed: {err}");
    }

    let paths = match AppPaths::from_config_with_origin(&root, origin, &config) {
        Ok(paths) => paths,
        Err(err) => {
            eprintln!("gguf-core: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = paths.ensure_dirs() {
        eprintln!("gguf-core: {err}");
        return ExitCode::FAILURE;
    }

    // Install the crash hook as early as possible: a panic during startup is
    // exactly the case a user cannot otherwise report.
    let crash_dir = gguf_engine_telemetry::crash::install(paths.logs());
    tracing::debug!(directory = %crash_dir.display(), "crash reporting enabled");

    if args.check {
        return check_mode(&paths);
    }

    run_daemon(&config, &paths).await
}

/// Startup report used by packaging/native-UI health checks and by tests.
fn check_mode(paths: &AppPaths) -> ExitCode {
    let database = match Database::open(paths.database_file()) {
        Ok(database) => database,
        Err(err) => {
            eprintln!("gguf-core: opening the database failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    let schema_version = match database.schema_version() {
        Ok(version) => version,
        Err(err) => {
            eprintln!("gguf-core: reading the schema version failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    let report = serde_json::json!({
        "ok": true,
        "core": gguf_engine_core::CORE_NAME,
        "version": gguf_engine_core::CORE_VERSION,
        "protocol_version": gguf_engine_protocol::PROTOCOL_VERSION,
        "data_root": paths.root().display().to_string(),
        "database": paths.database_file().display().to_string(),
        "schema_version": schema_version,
        "ipc_endpoint": gguf_engine_protocol::ipc_endpoint(paths.root()),
        "state": CoreState::Stopped.as_str(),
        "features": gguf_engine_protocol::features()
            .iter()
            .map(|f| serde_json::json!({
                "id": f.id,
                "status": f.status.as_str(),
                "reason": f.reason,
            }))
            .collect::<Vec<_>>(),
    });
    println!("{report}");
    ExitCode::SUCCESS
}

async fn run_daemon(config: &Config, paths: &AppPaths) -> ExitCode {
    let lock = match InstanceLock::acquire(paths.lock_file()) {
        Ok(lock) => lock,
        Err(LockError::Held { pid, .. }) => {
            tracing::error!(pid, "another core instance is already running");
            eprintln!(
                "gguf-core: another core instance is already running (pid {pid}). \
                 Use the UI or `gguf doctor` instead of starting a second daemon."
            );
            return ExitCode::from(EXIT_ALREADY_RUNNING);
        }
        Err(err) => {
            tracing::error!(error = %err, "failed to acquire the single instance lock");
            eprintln!("gguf-core: {err}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        pid = std::process::id(),
        data_root = %paths.root().display(),
        "core starting"
    );

    // --- database -----------------------------------------------------------
    let database = match Database::open(paths.database_file()) {
        Ok(database) => database,
        Err(err) => {
            tracing::error!(error = %err, "database unavailable");
            eprintln!("gguf-core: opening the database failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    let db_status = match database.status().await {
        Ok(status) => status,
        Err(err) => {
            tracing::error!(error = %err, "database status query failed");
            eprintln!("gguf-core: database status failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        schema_version = db_status.schema_version,
        sqlite = %db_status.sqlite_version,
        "database ready"
    );

    // --- lifecycle ----------------------------------------------------------
    let bus = EventBus::new(BusConfig {
        capacity: 1024,
        replay_limit: 256,
        label: "core".to_owned(),
    });
    let lifecycle = Lifecycle::new(bus.clone());
    let monitor = HealthMonitor::new();

    if let Err(err) = lifecycle.transition(CoreState::Starting) {
        tracing::error!(error = %err, "failed to enter the starting state");
        return ExitCode::FAILURE;
    }

    let lifecycle_handle = Arc::new(LifecycleHandle::new(CoreState::Starting));
    let config_issues = gguf_engine_config::check(config);
    let state = IpcState {
        bus: bus.clone(),
        lifecycle: lifecycle_handle.clone(),
        database: database.clone(),
        started_at_unix_ms: gguf_engine_core::health::unix_millis(),
        instance_id: uuid::Uuid::now_v7().to_string(),
        protocol_version: gguf_engine_protocol::PROTOCOL_VERSION,
        features: gguf_engine_protocol::features(),
        config: config.clone(),
        config_issues,
        config_file: paths.config_file().display().to_string(),
        config_file_exists: paths.config_file().exists(),
        data_root: paths.root().display().to_string(),
        models_dir: paths.models().display().to_string(),
        endpoint: gguf_engine_protocol::ipc_endpoint(paths.root()),
        hardware: std::sync::Arc::new(gguf_engine_ipc::services::HardwareCache::default()),
    };

    // --- IPC ----------------------------------------------------------------
    let server = match gguf_engine_ipc::serve(paths.root(), state).await {
        Ok(server) => server,
        Err(err) => {
            tracing::error!(error = %err, "IPC listener failed");
            eprintln!(
                "gguf-core: {}",
                gguf_engine_ipc::server::describe_bind_failure(
                    &gguf_engine_protocol::ipc_endpoint(paths.root()),
                    &err
                )
            );
            return ExitCode::from(EXIT_IPC_UNAVAILABLE);
        }
    };
    tracing::info!(endpoint = server.endpoint(), "IPC listening");

    // --- OpenAI-compatible gateway -------------------------------------------
    //
    // Started after IPC so it can relay through the same transport the UI uses.
    // A gateway failure is logged and survived: the core is still usable through
    // the CLI and the shells, and refusing to start over a busy port would be a
    // worse outcome than reporting it.
    let gateway = if config.api.enabled {
        let api_config = gguf_engine_api_server::ApiConfig {
            bind: format!("{}:{}", config.api.bind, config.effective_api_port())
                .parse()
                .unwrap_or_else(|_| {
                    std::net::SocketAddr::from(([127, 0, 0, 1], config.effective_api_port()))
                }),
            token: config.api.token.clone(),
            allow_browser_origins: false,
        };

        let backend = std::sync::Arc::new(gguf_engine_api_server::IpcBackend::new(paths.root()));
        // The media endpoints probe the non-LLM runtimes; when none is installed
        // they answer 503 with the probe's reason instead of failing obscurely.
        let media = std::sync::Arc::new(gguf_engine_api_server::LocalMedia::from_config(
            &config.multimodal,
        ));
        match gguf_engine_api_server::serve(api_config, backend, media).await {
            Ok(gateway) => {
                tracing::info!(address = %gateway.address(), "API gateway listening");
                Some(gateway)
            }
            Err(err) => {
                tracing::warn!(error = %err, "the API gateway did not start");
                eprintln!("gguf-core: API gateway not started: {err}");
                None
            }
        }
    } else {
        tracing::info!("the API gateway is disabled in the configuration");
        None
    };

    if let Err(err) = lifecycle.transition(CoreState::Ready) {
        tracing::error!(error = %err, "failed to become ready");
        let _ = server.shutdown().await;
        return ExitCode::FAILURE;
    }
    lifecycle_handle.set(CoreState::Ready);

    let components = vec![
        ComponentHealth::healthy("lifecycle"),
        ComponentHealth::healthy("ipc"),
        ComponentHealth::healthy("database"),
        ComponentHealth::healthy("event-bus"),
    ];
    let health = monitor.report(lifecycle.current(), bus.stats(), components);
    tracing::info!(health = %health.summary(), endpoint = server.endpoint(), "core ready");

    // --- serve --------------------------------------------------------------
    let interval = Duration::from_millis(config.core.health_interval_ms.max(100));
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("shutdown signal received");
                break;
            }
            _ = ticker.tick() => {
                let uptime_ms = monitor.uptime_ms();
                let _ = bus.publish(EventPayload::HealthTick {
                    uptime_ms,
                    subscribers: bus.subscriber_count(),
                });
            }
        }
    }

    // --- shutdown -----------------------------------------------------------
    // Stop the gateway before the IPC it depends on, so an in-flight request
    // gets a connection error rather than a half-closed pipe.
    if let Some(gateway) = gateway {
        tracing::info!(address = %gateway.address(), "stopping the API gateway");
        gateway.stop();
    }

    lifecycle_handle.set(CoreState::ShuttingDown);
    if let Err(err) = lifecycle.transition(CoreState::ShuttingDown) {
        tracing::error!(error = %err, "failed to enter the shutting-down state");
    }
    if let Err(err) = server.shutdown().await {
        tracing::error!(error = %err, "IPC shutdown failed");
    }
    if let Err(err) = lifecycle.transition(CoreState::Stopped) {
        tracing::error!(error = %err, "failed to enter the stopped state");
        return ExitCode::FAILURE;
    }
    lifecycle_handle.set(CoreState::Stopped);

    let stats = bus.stats();
    tracing::info!(
        published = stats.published,
        dropped = stats.dropped,
        "core stopped"
    );
    drop(lock);
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn clap_definition_is_valid() {
        Args::command().debug_assert();
    }

    #[test]
    fn check_mode_emits_json() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_config(tmp.path(), &Config::default()).unwrap();
        paths.ensure_dirs().unwrap();
        assert_eq!(check_mode(&paths), ExitCode::SUCCESS);
    }

    #[test]
    fn defaults_to_pretty_logs() {
        let args = Args::parse_from(["gguf-core"]);
        assert_eq!(args.log_format, LogFormat::Pretty);
    }

    #[test]
    fn check_mode_migrates_the_database() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_config(tmp.path(), &Config::default()).unwrap();
        paths.ensure_dirs().unwrap();
        assert_eq!(check_mode(&paths), ExitCode::SUCCESS);
        let db = Database::open(paths.database_file()).unwrap();
        assert_eq!(
            db.schema_version().unwrap(),
            gguf_engine_database::latest_version()
        );
    }
}
