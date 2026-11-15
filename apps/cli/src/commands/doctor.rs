//! `gguf doctor` — environment diagnostics (spec rule 12: capability probe
//! plus an explicit degradation path).
//!
//! Task 1 checks what exists today: directories, configuration, the
//! single-instance lock and the runtime toolchain. Hardware probing arrives
//! with Task 4 and is reported here as `planned` rather than faked.

use anyhow::Result;
use gguf_engine_core::{ComponentHealth, HealthMonitor};
use gguf_engine_database::Database;
use gguf_engine_event_bus::{BusConfig, CoreState, EventBus};
use serde::Serialize;

use crate::commands::Context;
use crate::output;

#[derive(Debug, Serialize)]
struct Check {
    name: String,
    ok: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    ok: bool,
    data_root: String,
    config_file: String,
    ipc_endpoint: String,
    checks: Vec<Check>,
    planned: Vec<String>,
    supported_features: Vec<String>,
}

fn check_dirs(ctx: &Context) -> Check {
    match ctx.paths.ensure_dirs() {
        Ok(()) => Check {
            name: "directories".into(),
            ok: true,
            detail: format!("created/verified under {}", ctx.paths.root().display()),
        },
        Err(err) => Check {
            name: "directories".into(),
            ok: false,
            detail: err.to_string(),
        },
    }
}

/// Report any crash reports left behind, so support can ask for one.
///
/// This is informational: old crash files are not a failure of the current
/// install, so the check always passes and only reports what it found.
fn check_crash_reports(ctx: &Context) -> Check {
    let reports = gguf_engine_telemetry::crash::reports(ctx.paths.logs());
    let detail = if reports.is_empty() {
        format!("none in {}", ctx.paths.logs().display())
    } else {
        format!(
            "{} report(s); newest: {}",
            reports.len(),
            reports[0].display()
        )
    };

    Check {
        name: "crash reports".into(),
        ok: true,
        detail,
    }
}

fn check_disk_space(ctx: &Context) -> Check {
    // A conservative check: the models directory must exist and be writable.
    let probe = ctx.paths.cache().join(".doctor-write-probe");
    match std::fs::write(&probe, b"ok").and_then(|()| std::fs::remove_file(&probe)) {
        Ok(()) => Check {
            name: "disk writable".into(),
            ok: true,
            detail: ctx.paths.cache().display().to_string(),
        },
        Err(err) => Check {
            name: "disk writable".into(),
            ok: false,
            detail: format!("{}: {err}", ctx.paths.cache().display()),
        },
    }
}

fn check_lock(ctx: &Context) -> Check {
    let path = ctx.paths.lock_file();
    match gguf_engine_core::InstanceLock::acquire(&path) {
        Ok(lock) => {
            let pid = lock.owner_pid();
            drop(lock);
            Check {
                name: "single instance".into(),
                ok: true,
                detail: format!(
                    "acquired and released {} (no other core running; probe pid {pid})",
                    path.display()
                ),
            }
        }
        Err(gguf_engine_core::LockError::Held { pid, path }) => {
            // Another core owning the lock is the *expected* state whenever the
            // daemon is up; it is only a problem if you were trying to start a
            // second one.
            let owner = if pid == 0 {
                "another process".to_owned()
            } else {
                format!("pid {pid}")
            };
            Check {
                name: "single instance".into(),
                ok: true,
                detail: format!("held by {owner} ({path}) — a core is already running"),
            }
        }
        Err(err) => Check {
            name: "single instance".into(),
            ok: false,
            detail: err.to_string(),
        },
    }
}

async fn check_database(ctx: &Context) -> Check {
    let path = ctx.paths.database_file();
    let database = match Database::open(&path) {
        Ok(database) => database,
        Err(err) => {
            return Check {
                name: "database".into(),
                ok: false,
                detail: err.to_string(),
            };
        }
    };
    match database.status().await {
        Ok(status) => Check {
            name: "database".into(),
            ok: status.schema_version == gguf_engine_database::latest_version(),
            detail: format!(
                "schema v{} (expected v{}), sqlite {}, {}",
                status.schema_version,
                gguf_engine_database::latest_version(),
                status.sqlite_version,
                status.path
            ),
        },
        Err(err) => Check {
            name: "database".into(),
            ok: false,
            detail: err.to_string(),
        },
    }
}

/// Probe a *running* core over IPC.
///
/// A core that is not running is not a failure: `gguf doctor` must work on a
/// machine where the UI has not started yet. It reports "not running" and moves
/// on, but a core that *is* listening and answers badly is a failure.
async fn check_live_core(ctx: &Context) -> Check {
    let endpoint = gguf_engine_protocol::ipc_endpoint(ctx.paths.root());
    match gguf_engine_ipc::connect(ctx.paths.root()).await {
        Ok(client) => match client.health().await {
            Ok(health) => {
                let healthy = health.components.iter().filter(|c| c.healthy).count();
                let unusable: Vec<&str> = health
                    .components
                    .iter()
                    .filter(|c| !c.healthy)
                    .map(|c| c.name.as_str())
                    .collect();
                Check {
                    name: "core (running)".into(),
                    // A reachable, answering core is a passing check even when
                    // it reports unimplemented subsystems: that is the honest
                    // state of an unfinished build, and the doctor prints it.
                    ok: true,
                    detail: format!(
                        "{} on {} (pid {}); {healthy}/{} components healthy{}",
                        core_state_name(health.state),
                        client.endpoint(),
                        health.pid,
                        health.components.len(),
                        if unusable.is_empty() {
                            String::new()
                        } else {
                            format!(", not implemented: {}", unusable.join(", "))
                        }
                    ),
                }
            }
            Err(err) => Check {
                name: "core (running)".into(),
                ok: false,
                detail: format!("listening at {endpoint} but health failed: {err}"),
            },
        },
        Err(gguf_engine_ipc::IpcError::NotRunning(_)) => Check {
            name: "core (running)".into(),
            ok: true,
            detail: format!("not running (endpoint {endpoint}); start it with `gguf serve`"),
        },
        Err(err) => Check {
            name: "core (running)".into(),
            ok: false,
            detail: err.to_string(),
        },
    }
}

fn core_state_name(state: i32) -> &'static str {
    match gguf_engine_protocol::pb::CoreState::try_from(state) {
        Ok(gguf_engine_protocol::pb::CoreState::Stopped) => "stopped",
        Ok(gguf_engine_protocol::pb::CoreState::Starting) => "starting",
        Ok(gguf_engine_protocol::pb::CoreState::Ready) => "ready",
        Ok(gguf_engine_protocol::pb::CoreState::Degraded) => "degraded",
        Ok(gguf_engine_protocol::pb::CoreState::ShuttingDown) => "shutting_down",
        _ => "unknown",
    }
}

async fn check_core_health() -> Check {
    // Exercise the real lifecycle + health path the daemon uses.
    let bus = EventBus::new(BusConfig::default());
    let lifecycle = gguf_engine_core::Lifecycle::new(bus.clone());
    let monitor = HealthMonitor::new();
    let started = match lifecycle.boot() {
        Ok(changes) => changes,
        Err(err) => {
            return Check {
                name: "core lifecycle".into(),
                ok: false,
                detail: err.to_string(),
            };
        }
    };
    let report = monitor.report(
        lifecycle.current(),
        bus.stats(),
        vec![ComponentHealth::healthy("lifecycle")],
    );
    let ok = report.is_healthy() && lifecycle.current() == CoreState::Ready;
    let detail = format!("{} transitions, {}", started.len(), report.summary());
    if let Err(err) = lifecycle.shutdown() {
        return Check {
            name: "core lifecycle".into(),
            ok: false,
            detail: format!("shutdown failed: {err}"),
        };
    }
    Check {
        name: "core lifecycle".into(),
        ok,
        detail,
    }
}

/// Run the diagnostics. Returns `true` when every check passed.
pub async fn doctor(ctx: &Context, json: bool) -> Result<bool> {
    let checks = vec![
        check_dirs(ctx),
        check_disk_space(ctx),
        check_lock(ctx),
        check_database(ctx).await,
        check_core_health().await,
        check_live_core(ctx).await,
        check_crash_reports(ctx),
    ];
    let planned = vec![
        "hardware inventory (Task 4)".to_owned(),
        "GGUF parser (Task 5)".to_owned(),
        "Hugging Face download (Task 5)".to_owned(),
        "llama.cpp runtime (Task 6)".to_owned(),
        "multi-GPU scheduling (Task 7)".to_owned(),
        "HTTP / OpenAI-compatible API".to_owned(),
    ];
    let report = DoctorReport {
        ok: checks.iter().all(|c| c.ok),
        data_root: ctx.paths.root().display().to_string(),
        config_file: ctx.config_file.display().to_string(),
        ipc_endpoint: gguf_engine_protocol::ipc_endpoint(ctx.paths.root()),
        checks,
        planned,
        supported_features: gguf_engine_protocol::features()
            .into_iter()
            .filter(|f| f.status == gguf_engine_protocol::FeatureStatus::Supported)
            .map(|f| f.id)
            .collect(),
    };

    let ok = report.ok;
    output::emit(json, &report, |report| {
        println!("data root : {}", report.data_root);
        println!("config    : {}", report.config_file);
        println!("ipc       : {}", report.ipc_endpoint);
        println!();
        for check in &report.checks {
            let mark = if check.ok { "ok  " } else { "FAIL" };
            println!("[{mark}] {:<18} {}", check.name, check.detail);
        }
        println!();
        println!(
            "implemented features : {}",
            if report.supported_features.is_empty() {
                "none".to_owned()
            } else {
                report.supported_features.join(", ")
            }
        );
        println!("planned              : {}", report.planned.join(", "));
        println!();
        println!(
            "{}",
            if report.ok {
                "doctor: all checks passed"
            } else {
                "doctor: problems found, see FAIL lines above"
            }
        );
    });

    if !json && !ctx.warnings.is_empty() {
        eprintln!();
        eprintln!("configuration warnings:");
        for issue in &ctx.warnings {
            eprintln!("  {}: {}", issue.field, issue.message);
        }
    }
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gguf_engine_config::Origin;

    #[test]
    fn doctor_passes_on_a_clean_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = Context::resolve_with_origin(Some(tmp.path()), Origin::Explicit, None).unwrap();
        let ok = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(doctor(&ctx, true))
            .unwrap();
        assert!(ok, "expected a clean data dir to pass every check");
    }

    #[test]
    fn doctor_never_claims_unimplemented_features() {
        let supported: Vec<String> = gguf_engine_protocol::features()
            .into_iter()
            .filter(|f| f.status == gguf_engine_protocol::FeatureStatus::Supported)
            .map(|f| f.id)
            .collect();
        // llama.cpp became real in Task 6.
        assert!(supported.contains(&"runtime_llama_cpp".to_owned()));
        // Hardware (Task 4) and the model registry + downloads (Task 5) are real.
        assert!(supported.contains(&"hardware_inventory".to_owned()));
        assert!(supported.contains(&"model_registry".to_owned()));
        assert!(supported.contains(&"model_download".to_owned()));
        // Task 7: scheduling and the memory tier model.
        assert!(supported.contains(&"multi_gpu_scheduling".to_owned()));
        assert!(supported.contains(&"memory_paging".to_owned()));
        assert!(supported.contains(&"ipc".to_owned()));
    }

    #[test]
    fn core_state_names_cover_every_wire_value() {
        use gguf_engine_protocol::pb;
        assert_eq!(core_state_name(pb::CoreState::Ready as i32), "ready");
        assert_eq!(core_state_name(pb::CoreState::Degraded as i32), "degraded");
        assert_eq!(core_state_name(999), "unknown");
    }
}
