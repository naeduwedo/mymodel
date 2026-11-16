//! End-to-end tests for the `gguf` CLI binary.
//!
//! These run the real executable, which is what a user and the packaging
//! pipeline depend on. Every test uses an isolated `--data-dir` so nothing
//! touches the developer's application data.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// The path to a workspace binary.
///
/// `CARGO_BIN_EXE_<name>` is only defined for `[[bin]]` targets of the package
/// being tested, and these binaries live in other packages, so it cannot be used.
/// Instead this walks up from the test executable: Cargo has placed the harness
/// in `deps/`, and now in a `build/.../out/` directory, so the search must not
/// assume one layout. Finding the binary is what matters, not where it sits.
fn bin_path(name: &str) -> std::path::PathBuf {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };

    let start = std::env::current_exe().expect("current executable path");
    let mut looked = Vec::new();
    for ancestor in start.ancestors().skip(1).take(6) {
        let candidate = ancestor.join(&exe);
        if candidate.is_file() {
            return candidate;
        }
        looked.push(candidate);
    }

    panic!(
        "binary {exe} not found; looked in {looked:?}. Run the suite with 'cargo test --workspace'."
    );
}

struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Invoke a binary with `--data-dir`. Arguments declared as global in clap are
/// accepted either before or after the subcommand, so the order used here is
/// the one a user would type.
fn run(bin: &str, args: &[&str], data_dir: &Path) -> Run {
    let output = Command::new(bin_path(bin))
        .args(args)
        .arg("--data-dir")
        .arg(data_dir)
        .output()
        .expect("failed to spawn the binary");
    Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn json(run: &Run) -> Value {
    serde_json::from_str(&run.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout was not JSON ({err}):\nstdout: {}\nstderr: {}",
            run.stdout, run.stderr
        )
    })
}

#[test]
fn version_is_machine_readable_and_names_the_protocol() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run("gguf", &["version", "--json"], tmp.path());
    assert_eq!(run.code, Some(0), "stderr: {}", run.stderr);
    let value = json(&run);
    assert_eq!(value["cli"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["protocol"], gguf_engine_protocol::PROTOCOL_VERSION);
    assert!(value["os"].is_string());
}

#[test]
fn doctor_passes_on_an_empty_data_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run("gguf", &["doctor", "--json"], tmp.path());
    assert_eq!(
        run.code,
        Some(0),
        "stdout: {}\nstderr: {}",
        run.stdout,
        run.stderr
    );
    let value = json(&run);
    assert_eq!(value["ok"], true);
    let checks = value["checks"].as_array().expect("checks array");
    assert!(checks.len() >= 4);
    for check in checks {
        assert_eq!(check["ok"], true, "check failed: {check}");
    }
    // llama.cpp (Task 6) is now advertised; hardware/download/registry too.
    let features = value["supported_features"].as_array().unwrap();
    for now_supported in [
        "ipc",
        "database",
        "hardware_inventory",
        "model_registry",
        "runtime_llama_cpp",
    ] {
        assert!(
            features.iter().any(|f| f == now_supported),
            "{now_supported} must be supported"
        );
    }
    // ... and the planned work is stated explicitly.
    assert!(!value["planned"].as_array().unwrap().is_empty());
}

#[test]
fn doctor_creates_the_runtime_directory_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run("gguf", &["doctor"], tmp.path());
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    for dir in [
        "models",
        "cache",
        "runtimes",
        "logs",
        "database",
        "cache/downloads",
    ] {
        assert!(
            tmp.path().join(dir).is_dir(),
            "{dir} should have been created"
        );
    }
}

#[test]
fn config_defaults_match_the_specification() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run("gguf", &["config", "--defaults", "--json"], tmp.path());
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let value = json(&run);
    assert_eq!(value["config"]["core"]["api_port"], 18888);
    assert_eq!(value["config"]["core"]["grpc_port"], 18889);
    assert_eq!(
        value["config"]["memory"]["gpu_reserve_bytes"],
        2_147_483_648u64
    );
    assert_eq!(value["config"]["api"]["bind"], "127.0.0.1");
    assert_eq!(value["data_root_origin"], "explicit");
    assert_eq!(value["issues"].as_array().unwrap().len(), 0);
}

#[test]
fn invalid_configuration_fails_with_a_field_level_message() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.toml");
    std::fs::write(&cfg, "[core]\napi_port = 18889\ngrpc_port = 18889\n").unwrap();

    let output = Command::new(bin_path("gguf"))
        .args(["doctor"])
        .arg("--data-dir")
        .arg(tmp.path())
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("grpc_port") && stderr.contains("api_port"),
        "expected a field-level error, got: {stderr}"
    );
}

#[test]
fn unknown_configuration_keys_are_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("config.toml"),
        "[core]\napi_port = 18888\nmystery = 1\n",
    )
    .unwrap();
    let output = Command::new(bin_path("gguf"))
        .args(["config"])
        .arg("--data-dir")
        .arg(tmp.path())
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("mystery"), "got: {stderr}");
}

#[test]
fn core_daemon_check_mode_reports_its_capabilities() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run("gguf-core", &["--check"], tmp.path());
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let value = json(&run);
    assert_eq!(value["ok"], true);
    assert_eq!(value["core"], gguf_engine_core::CORE_NAME);
    assert_eq!(
        value["protocol_version"],
        gguf_engine_protocol::PROTOCOL_VERSION
    );
    let features = value["features"].as_array().unwrap();
    assert!(
        features.iter().all(|f| f != "runtime_llama_cpp"),
        "must not advertise an unimplemented runtime"
    );
}

#[test]
fn worker_refuses_to_start_without_a_runtime_adapter() {
    let tmp = tempfile::tempdir().unwrap();
    let run = run("gguf-worker", &["llama.cpp"], tmp.path());
    assert_eq!(run.code, Some(4), "worker must fail closed");
    assert!(run.stderr.contains("no runtime adapter"));
    assert!(run.stderr.contains("llama.cpp"));
}

#[test]
fn data_dir_environment_variable_is_honoured() {
    let tmp = tempfile::tempdir().unwrap();
    let output = Command::new(bin_path("gguf"))
        .args(["config", "--json"])
        .env("GGUF_AI_ENGINE_DATA_DIR", tmp.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data_root_origin"], "environment");
    assert_eq!(
        std::path::PathBuf::from(value["data_root"].as_str().unwrap()),
        tmp.path()
    );
}
