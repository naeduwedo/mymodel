//! End-to-end test of the real daemon binary over the real platform transport.
//!
//! **Build the binaries first.** These tests spawn `target/<profile>/gguf-core`,
//! and `cargo test` does not refresh that file: it builds test harnesses into
//! `deps/` instead. Run `cargo build --workspace` before `cargo test`, or a stale
//! binary will be tested and the result will depend on build order.
//!
//! Nothing here is mocked: the test starts `gguf-core`, connects over the named
//! pipe (Windows) or Unix socket (Linux/macOS) using the same client the CLI and
//! the native UIs use, reads health, streams events, and then shuts the core
//! down. It is the test that would catch a broken ACL, a stale socket or a
//! protocol drift between core and client.

use std::path::{Path, PathBuf};

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio_stream::StreamExt as _;

/// A running daemon that is killed if the test panics.
struct Daemon {
    child: Child,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn bin(name: &str) -> PathBuf {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };

    // Walk up: harnesses live in `deps/` or in a `build/.../out/` directory
    // depending on the Cargo version, so neither layout may be assumed.
    let start = std::env::current_exe().expect("test executable path");
    let mut looked = Vec::new();
    for ancestor in start.ancestors().skip(1).take(6) {
        let candidate = ancestor.join(&exe);
        if candidate.is_file() {
            return candidate;
        }
        looked.push(candidate);
    }

    panic!("could not find {exe}; looked in {looked:?}");
}

fn start_daemon(data_root: &Path) -> Daemon {
    let child = Command::new(bin("gguf-core"))
        .arg("--data-dir")
        .arg(data_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawning gguf-core");
    Daemon { child }
}

/// Connect to the daemon, retrying until it is listening.
///
/// A raw "is the pipe open?" probe is not used here: on Windows every probe
/// consumes one pipe instance, which would compete with the connection the
/// test actually wants. Retrying the real client is both simpler and closer to
/// what a UI does on startup.
async fn connect_with_retry(data_root: &Path, timeout: Duration) -> gguf_engine_ipc::CoreClient {
    let deadline = Instant::now() + timeout;
    let mut last = String::from("no attempt was made");
    while Instant::now() < deadline {
        match gguf_engine_ipc::connect(data_root).await {
            Ok(client) => return client,
            Err(err) => {
                last = err.to_string();
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    panic!("could not connect to the core within {timeout:?}: {last}");
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

#[test]
fn daemon_serves_health_over_the_platform_transport() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let _daemon = start_daemon(tmp.path());

    runtime().block_on(async {
        let client = connect_with_retry(tmp.path(), Duration::from_secs(20)).await;
        let health = client.health().await.expect("health over IPC");
        assert_eq!(
            health.state,
            gguf_engine_protocol::pb::CoreState::Ready as i32
        );
        assert_eq!(health.pid, _daemon.child.id());
        assert_eq!(
            health.version,
            env!("CARGO_PKG_VERSION"),
            "the core must report its own version"
        );

        // Implemented subsystems report healthy; unbuilt ones report unhealthy
        // *with a reason* rather than being silently absent.
        let ipc = health
            .components
            .iter()
            .find(|c| c.name == "ipc")
            .expect("ipc component");
        assert!(ipc.healthy);
        let database = health
            .components
            .iter()
            .find(|c| c.name == "database")
            .expect("database component");
        assert!(database.healthy, "{database:?}");
        // The schema version moves with every migration; assert the shape, not a
        // literal, so adding a migration does not break this test.
        assert!(
            database.detail.contains("schema v"),
            "the database check must report its schema version: {}",
            database.detail
        );

        // One honest gap remains: image generation, video, audio and music have
        // no runtime adapter. It must be listed as unhealthy *with a reason*,
        // and nothing else may be.
        let unimplemented: Vec<&str> = health
            .components
            .iter()
            .filter(|c| !c.healthy && c.name != "ipc")
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(
            unimplemented,
            vec!["music_generation"],
            "only music has no runtime adapter in this build"
        );

        let gap = health
            .components
            .iter()
            .find(|c| c.name == "music_generation")
            .expect("the gap is listed, not hidden");
        assert!(
            !gap.detail.is_empty(),
            "an unimplemented capability must explain itself"
        );
    });
}

#[test]
fn daemon_streams_events_and_survives_a_disconnect() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let _daemon = start_daemon(tmp.path());

    runtime().block_on(async {
        let client = connect_with_retry(tmp.path(), Duration::from_secs(20)).await;

        // First subscriber: read the boot transitions from the replay ring.
        let mut first = Box::pin(client.subscribe(0).await.expect("subscribe"));
        let boot = tokio::time::timeout(Duration::from_secs(5), first.next())
            .await
            .expect("a replayed boot event")
            .expect("stream open")
            .expect("event");
        assert!(boot.seq >= 1);
        assert!(
            boot.payload_json.contains("core_state_changed")
                || boot.payload_json.contains("health_tick"),
            "unexpected payload: {}",
            boot.payload_json
        );
        let seen_seq = boot.seq;
        drop(first);

        // Second subscriber resumes from where the first left off: a UI
        // reconnecting must not miss or repeat events.
        let mut second = Box::pin(client.subscribe(seen_seq).await.expect("resubscribe"));
        let next = tokio::time::timeout(Duration::from_secs(5), second.next())
            .await
            .expect("a live event")
            .expect("stream open")
            .expect("event");
        assert!(
            next.seq > seen_seq,
            "resume must only deliver newer events ({} <= {seen_seq})",
            next.seq
        );
    });
}

#[test]
fn a_second_daemon_refuses_to_start_and_the_first_keeps_serving() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let _daemon = start_daemon(tmp.path());

    // Bounded wait on purpose. `.status()` blocks for ever if the guard fails,
    // and a test that hangs takes the whole CI job down with it instead of
    // saying what went wrong.
    let mut second = Command::new(bin("gguf-core"))
        .arg("--data-dir")
        .arg(tmp.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawning a second core");

    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = second.try_wait().expect("polling the second core") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = second.kill();
            let output = second
                .wait_with_output()
                .expect("collecting the second core");
            panic!(
                "the second core kept running against the same data root; \
                 the single-instance lock is not enforced on this platform. stderr: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    assert_eq!(
        status.code(),
        Some(3),
        "a second core must exit with the 'already running' code"
    );

    // The first core is unaffected by the failed attempt.
    runtime().block_on(async {
        let client = connect_with_retry(tmp.path(), Duration::from_secs(5)).await;
        client.ping().await.expect("the first core still answers");
    });
}

#[test]
fn doctor_reports_the_running_core() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let _daemon = start_daemon(tmp.path());

    let output = Command::new(bin("gguf"))
        .args(["doctor", "--json", "--data-dir"])
        .arg(tmp.path())
        .output()
        .expect("running gguf doctor");
    assert_eq!(output.status.code(), Some(0));

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("doctor emits JSON");
    assert_eq!(report["ok"], true);
    let running = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "core (running)")
        .expect("doctor probes the live core");
    assert_eq!(running["ok"], true);
    assert!(
        running["detail"].as_str().unwrap().contains("ready"),
        "{}",
        running["detail"]
    );
}
