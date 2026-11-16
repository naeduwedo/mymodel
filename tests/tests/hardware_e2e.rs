//! End-to-end test: hardware inventory over the real platform transport.
//!
//! This drives the real daemon, which spawns the real vendor tools. It therefore
//! asserts *structure and honesty*, not specific hardware: the machine may be
//! CPU-only in CI. What must hold everywhere:
//!
//! * CPU/memory/storage are always reported;
//! * a GPU result is either real or explicitly unavailable with a reason;
//! * topology is never claimed as verified;
//! * no ANSI escapes leak into user-facing text.
//!
//! The test spawns gguf-core from target/<profile>/, so the binaries must be
//! up to date. cargo test --workspace --all-targets builds them first; running
//! only cargo test -p gguf-engine-tests after changing the daemon does not, and
//! would silently exercise the previous build.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use gguf_engine_protocol::pb;

struct Daemon {
    child: Child,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bin(name: &str) -> std::path::PathBuf {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };

    // Cargo has placed test harnesses in `deps/` and, more recently, in a
    // `build/.../out/` directory, so walk up rather than assume a layout.
    let start = std::env::current_exe().expect("test executable path");
    let mut looked = Vec::new();
    for ancestor in start.ancestors().skip(1).take(6) {
        let candidate = ancestor.join(&exe);
        if candidate.is_file() {
            return candidate;
        }
        looked.push(candidate);
    }

    panic!(
        "could not find {exe}; looked in {looked:?}. Run the suite with 'cargo test --workspace'."
    );
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

async fn connect_with_retry(data_root: &Path, timeout: Duration) -> gguf_engine_ipc::CoreClient {
    let deadline = Instant::now() + timeout;
    let mut last = String::from("no attempt");
    while Instant::now() < deadline {
        match gguf_engine_ipc::connect(data_root).await {
            Ok(client) => return client,
            Err(err) => {
                last = err.to_string();
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    panic!("core did not start listening: {last}");
}

/// A port no other test in this process will use.
///
/// Asking the OS for a free port and immediately releasing it leaves a window: a
/// parallel daemon can take it before this one binds, which made the ubuntu job
/// fail intermittently. Handing out distinct numbers from a counter removes the
/// race between the tests themselves; the probe below only guards against
/// something outside the suite already listening.
fn free_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(19200);

    for _ in 0..500 {
        let port = NEXT.fetch_add(1, Ordering::Relaxed);
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free port in the test range");
}

fn start_daemon(data_root: &Path) -> Daemon {
    // Parallel tests each need their own gateway port; see `free_port`.
    std::fs::write(
        data_root.join(gguf_engine_config::CONFIG_FILE_NAME),
        format!("[core]\napi_port = {}\n", free_port()),
    )
    .expect("writing a test config");

    let child = Command::new(bin("gguf-core"))
        .arg("--data-dir")
        .arg(data_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawning gguf-core");
    Daemon { child }
}

#[test]
fn hardware_inventory_is_served_over_ipc() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let _daemon = start_daemon(tmp.path());

    runtime().block_on(async {
        let client = connect_with_retry(tmp.path(), Duration::from_secs(20)).await;
        let report = client.hardware().await.expect("hardware over IPC");

        // CPU, memory and storage come from the OS and are always available.
        let cpu = report.cpu.expect("cpu");
        assert!(cpu.logical_cores > 0);
        assert!(!cpu.architecture.is_empty());
        assert!(
            cpu.physical_cores <= cpu.logical_cores,
            "physical cores must never exceed logical cores"
        );

        let memory = report.memory.expect("memory");
        assert!(memory.total_bytes > 0);
        assert!(memory.available_bytes <= memory.total_bytes);

        assert!(!report.storage.is_empty(), "at least one volume");
        assert!(report.collected_at_unix_ms > 0);

        // GPU: either real devices, or an explicit status with a reason.
        if report.gpus.is_empty() {
            assert_ne!(
                report.status,
                pb::CapabilityStatus::Supported as i32,
                "an empty GPU list must not be reported as supported"
            );
            assert!(
                !report.unavailable_reason.is_empty(),
                "an empty GPU list must explain itself"
            );
        } else {
            for gpu in &report.gpus {
                assert!(!gpu.name.is_empty());
                assert!(!gpu.vendor.is_empty());
                assert!(!gpu.backend.is_empty());
                if let (Some(total), Some(free)) = (gpu.total_vram_bytes, gpu.free_vram_bytes) {
                    assert!(free <= total, "free VRAM must not exceed total");
                }
            }
        }

        // Topology: never claimed as measured.
        if let Some(topology) = report.topology.as_ref() {
            for link in &topology.links {
                assert_ne!(
                    link.p2p_tested,
                    pb::CapabilityStatus::Supported as i32,
                    "P2P must not be reported as tested before a benchmark exists (spec §5.3)"
                );
                assert!(link.measured_bandwidth_bytes_sec.is_none());
            }
        }

        // No terminal escape sequences may reach a UI or a log.
        assert!(!report.unavailable_reason.contains('\u{1b}'));
        if let Some(topology) = &report.topology {
            assert!(!topology.unavailable_reason.contains('\u{1b}'));
        }
    });
}

#[test]
fn overview_includes_a_hardware_summary() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let _daemon = start_daemon(tmp.path());

    runtime().block_on(async {
        let client = connect_with_retry(tmp.path(), Duration::from_secs(20)).await;
        let snapshot = client.snapshot().await.expect("snapshot");
        let overview = snapshot.overview.expect("overview");

        for id in ["hw.cpu", "hw.memory", "hw.gpu"] {
            let tile = overview
                .tiles
                .iter()
                .find(|t| t.id == id)
                .unwrap_or_else(|| panic!("missing tile {id}"));
            assert!(!tile.value.is_empty(), "{id} has no value");
            assert!(!tile.value.contains('\u{1b}'));
        }

        // The capability list must now advertise hardware as supported.
        let hardware = overview
            .features
            .iter()
            .find(|f| f.id == "hardware_inventory")
            .expect("hardware_inventory row");
        assert_eq!(hardware.status, pb::CapabilityStatus::Supported as i32);
        assert!(hardware.reason.is_empty());
    });
}
