//! Documentation and repository hygiene tests.
//!
//! Spec §18 requires "documentation matches actual behaviour". These tests fail
//! the build when a documented example drifts from the code.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests/ has a parent")
        .to_path_buf()
}

#[test]
fn example_configuration_parses_and_matches_the_defaults() {
    let path = workspace_root().join("configs").join("config.example.toml");
    let text = std::fs::read_to_string(&path).expect("configs/config.example.toml exists");
    let parsed = gguf_engine_config::Config::from_toml_str(&text).expect("example config parses");
    assert_eq!(
        parsed,
        gguf_engine_config::Config::default(),
        "configs/config.example.toml documents values that differ from the defaults"
    );
    gguf_engine_config::validate(&parsed).expect("example config is valid");
}

#[test]
fn documented_config_example_in_the_spec_still_parses() {
    // The specification's §11 example uses relative "./models"-style paths;
    // it must keep working for users who copy it.
    let text = std::fs::read_to_string(workspace_root().join("GGUF-AI-Engine-Engineering-Spec.md"))
        .expect("spec is part of the repository");
    let start = text
        .find("[core]\nhost = \"127.0.0.1\"")
        .expect("spec contains the config example");
    let end = text[start..]
        .find("\n\u{0060}\u{0060}\u{0060}")
        .map(|offset| start + offset)
        .expect("config example is fenced");
    let toml = &text[start..end];

    let parsed = gguf_engine_config::Config::from_toml_str(toml).expect("spec example parses");
    assert_eq!(parsed.core.api_port, 18888);
    assert_eq!(parsed.memory.gpu_reserve_bytes, 2_147_483_648);

    // And the "./" prefix in the spec must resolve inside the data root.
    let tmp = tempfile::tempdir().unwrap();
    let paths = gguf_engine_config::AppPaths::from_config(tmp.path(), &parsed)
        .expect("spec paths stay inside the data root");
    assert!(paths.models().starts_with(tmp.path()));
}

#[test]
fn every_workspace_member_is_documented() {
    let root = workspace_root();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("[workspace.lints.rust]"));
    assert!(manifest.contains("unsafe_code = \"forbid\""));

    let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
    for dir in [
        "apps/core-daemon",
        "apps/cli",
        "apps/runtime-worker",
        "tests",
    ] {
        assert!(
            root.join(dir).join("Cargo.toml").is_file(),
            "{dir} is missing its manifest"
        );
    }
    for name in [
        "crates/config",
        "crates/event-bus",
        "crates/engine-core",
        "crates/protocol",
    ] {
        assert!(readme.contains(name), "README does not mention {name}");
    }

    for adr in 1..=9 {
        let prefix = format!("ADR-{adr:03}");
        let found = std::fs::read_dir(root.join("docs").join("adr"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with(&prefix));
        assert!(found, "docs/adr is missing {prefix}");
    }
}

#[test]
fn planned_crates_declare_their_status() {
    // Guards against a crate being wired into the product while still being a
    // shell, and against an implemented crate still advertising itself as
    // planned (spec rule 4: no fake features, in either direction).
    let root = workspace_root();
    let implemented = [
        "config",
        "event-bus",
        "engine-core",
        "protocol",
        "telemetry",
        "database",
        "security",
        "ipc",
        "hardware",
        "gguf",
        "model-hub",
        "runtime-manager",
        "runtime-llama-cpp",
        "gpu-manager",
        "memory-manager",
        "api-server",
    ];
    // Every crate is implemented; this list is intentionally empty and the loop
    // below is a no-op kept so the rule stays expressed.
    let planned: [&str; 0] = [];

    for name in implemented {
        let text = read_lib(&root, name);
        assert!(
            !text.contains("STATUS: Status = Status::Planned"),
            "{name} is implemented but still declares STATUS = Planned"
        );
    }

    for name in planned {
        let text = read_lib(&root, name);
        assert!(
            text.contains("STATUS: Status = Status::Planned"),
            "{name} has no implementation and must declare STATUS = Planned"
        );
    }
}

fn read_lib(root: &Path, crate_name: &str) -> String {
    let lib = root
        .join("crates")
        .join(crate_name)
        .join("src")
        .join("lib.rs");
    std::fs::read_to_string(&lib).unwrap_or_else(|err| panic!("reading {}: {err}", lib.display()))
}
