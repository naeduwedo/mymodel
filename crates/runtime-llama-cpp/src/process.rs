//! Worker process supervision: spawn, readiness, capture, kill (spec 3.3, rule 9).
//!
//! The worker is llama-server itself. The supervisor must: capture stdout/stderr,
//! wait for readiness without blocking forever, and kill gracefully then hard.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};
use tracing::{info, warn};

use gguf_engine_runtime_manager::RuntimeError;

use crate::client;

/// How long to wait for the worker to report healthy.
pub const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// How often to poll /health while waiting.
const READY_POLL: Duration = Duration::from_millis(250);

/// A supervised llama-server process.
#[derive(Debug)]
pub struct Worker {
    /// The tokio child process.
    child: Child,
    /// Base URL, e.g. http://127.0.0.1:PORT.
    pub base_url: String,
    /// Last stderr lines, for diagnostics.
    pub stderr_tail: String,
}

impl Worker {
    /// Process id.
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }
}

/// Spawn llama-server for a model and wait until it is ready.
pub async fn spawn(
    server_path: &Path,
    model_path: &Path,
    port: u16,
    gpu_layers: u32,
    context_size: u32,
    stderr_file: &Path,
) -> Result<Worker, RuntimeError> {
    let base_url = format!("http://127.0.0.1:{port}");

    let stderr =
        std::fs::File::create(stderr_file).map_err(|err| RuntimeError::Start(err.to_string()))?;

    let child = Command::new(server_path)
        .arg("--model")
        .arg(model_path)
        .arg("--port")
        .arg(port.to_string())
        .arg("--n-gpu-layers")
        .arg(gpu_layers.to_string())
        .arg("--ctx-size")
        .arg(context_size.to_string())
        .arg("--host")
        .arg("127.0.0.1")
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| RuntimeError::Start(err.to_string()))?;

    let mut worker = Worker {
        child,
        base_url,
        stderr_tail: String::new(),
    };

    info!(pid = ?worker.pid(), base = %worker.base_url, model = %model_path.display(), "llama-server spawned");

    // Wait for readiness, but detect an early exit instead of polling a corpse.
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        if let Some(status) = worker
            .child
            .try_wait()
            .map_err(|err| RuntimeError::Start(err.to_string()))?
        {
            let stderr_tail = read_tail(stderr_file);
            return Err(RuntimeError::WorkerExited {
                code: status.code(),
                stderr: stderr_tail,
            });
        }

        if client::healthy(&worker.base_url).await {
            return Ok(worker);
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(RuntimeError::NotReady {
                timeout_ms: READY_TIMEOUT.as_millis() as u64,
            });
        }

        tokio::time::sleep(READY_POLL).await;
    }
}

/// Read the last few lines of a file, for diagnostics.
fn read_tail(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(10);
    lines[start..].join("\n")
}
/// Stop the worker: graceful termination, then a hard kill after a grace period.
pub async fn shutdown(mut worker: Worker) -> Result<(), RuntimeError> {
    let Some(pid) = worker.child.id() else {
        let _ = worker.child.kill().await;
        return Ok(());
    };

    info!(pid, "stopping llama-server");

    let _ = worker.child.kill().await;
    let grace = Duration::from_secs(5);
    match tokio::time::timeout(grace, worker.child.wait()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(RuntimeError::Start(err.to_string())),
        Err(_elapsed) => {
            warn!(pid, "worker ignored termination; killing hard");
            let _ = worker.child.start_kill();
            let _ = worker.child.wait().await;
            Ok(())
        }
    }
}
