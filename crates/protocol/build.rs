//! Generates Rust types and gRPC stubs from `proto/*.proto`.
//!
//! `protoc` is not guaranteed to exist on a developer machine or a CI runner
//! (and the Windows MSVC toolchain does not ship one), so the build uses
//! Google's prebuilt binary through `protoc-bin-vendored`. The bundled version
//! is pinned by Cargo, which keeps generated output reproducible across
//! platforms.

use std::path::PathBuf;

use prost::Message as _;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root: PathBuf = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("proto"))
        .ok_or("cannot locate the workspace proto/ directory")?;

    let protos = [
        "common.proto",
        "health.proto",
        "events.proto",
        "hardware.proto",
        "gpu.proto",
        "memory.proto",
        "model.proto",
        "runtime.proto",
        "inference.proto",
        "task.proto",
        "ui.proto",
    ];

    for proto in protos {
        println!(
            "cargo:rerun-if-changed={}",
            proto_root.join(proto).display()
        );
    }

    let files: Vec<PathBuf> = protos.iter().map(|p| proto_root.join(p)).collect();

    // The workspace forbids `unsafe`, and `std::env::set_var` (the only way to
    // point prost-build at a specific compiler) is unsafe in edition 2024
    // because it races with concurrent readers. Instead we run the vendored
    // compiler ourselves to produce a FileDescriptorSet and let prost-build
    // generate code from that, which also guarantees the compiler version on
    // every machine.
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let descriptor_path = PathBuf::from(std::env::var("OUT_DIR")?).join("gguf.bin");

    let output = std::process::Command::new(&protoc)
        .arg("--include_imports")
        .arg("--include_source_info")
        .arg(format!(
            "--descriptor_set_out={}",
            descriptor_path.display()
        ))
        .arg(format!("--proto_path={}", proto_root.display()))
        .args(&files)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "protoc failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let descriptors = std::fs::read(&descriptor_path)?;

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .emit_rerun_if_changed(false)
        // Keep generated modules doc-free but gated: missing_docs is denied in
        // the crate root, generated code is excluded there instead.
        .compile_fds(prost_types::FileDescriptorSet::decode(descriptors.as_slice())?)?;

    Ok(())
}
