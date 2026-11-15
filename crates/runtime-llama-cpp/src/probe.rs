//! Probing an installed llama.cpp binary (spec 8.1: capability probe).
//!
//! The probe is read-only: it runs --list-devices and inspects the binary
//! directory for backend DLLs. It never loads a model.

use std::path::{Path, PathBuf};
use std::process::Command;

use gguf_engine_hardware::ComputeBackend;
use gguf_engine_runtime_manager::{CapabilityStatus, RuntimeError};
use tracing::{debug, warn};

/// Where llama.cpp binaries are looked for, in order.
pub const DEFAULT_SEARCH_DIRS: &[&str] = &["E:/llama.cpp", "C:/llama.cpp"];

/// Environment variable overriding the binary directory.
pub const BINARY_ENV: &str = "GGUF_LLAMA_CPP_DIR";

/// What a probe learned about one installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    /// Directory containing the binaries.
    pub directory: PathBuf,
    /// Full path to llama-server.
    pub server_path: PathBuf,
    /// Backends detected from DLLs present next to the binary.
    pub backends: Vec<ComputeBackend>,
    /// Whether at least one CUDA device is visible.
    pub cuda_devices: usize,
}

/// Locate llama-server, honouring the override env var first.
pub fn locate() -> Result<ProbeResult, RuntimeError> {
    let directory = locate_directory()?;
    let server_path = directory.join(server_name());
    if !server_path.is_file() {
        return Err(RuntimeError::NotFound(server_path.display().to_string()));
    }

    let backends = detect_backends(&directory);
    let cuda_devices = list_devices(&server_path);

    debug!(directory = %directory.display(), ?backends, cuda_devices, "probed llama.cpp");

    Ok(ProbeResult {
        directory,
        server_path,
        backends,
        cuda_devices,
    })
}

fn locate_directory() -> Result<PathBuf, RuntimeError> {
    if let Ok(override_dir) = std::env::var(BINARY_ENV) {
        let path = PathBuf::from(&override_dir);
        if path.is_dir() {
            return Ok(path);
        }
        warn!(env = BINARY_ENV, %override_dir, "override directory does not exist");
    }

    for name in [server_name(), "llama-server"] {
        if let Some(parent) = which(name).and_then(|path| path.parent().map(|p| p.to_path_buf())) {
            return Ok(parent);
        }
    }

    for dir in DEFAULT_SEARCH_DIRS {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            return Ok(path);
        }
    }

    Err(RuntimeError::NotFound(
        "llama-server not found; set GGUF_LLAMA_CPP_DIR to its directory".to_owned(),
    ))
}

fn server_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn detect_backends(directory: &Path) -> Vec<ComputeBackend> {
    let names: Vec<String> = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default();

    let mut backends = Vec::new();
    if names
        .iter()
        .any(|name| name.contains("ggml-cuda") || name.contains("cublas"))
    {
        backends.push(ComputeBackend::Cuda);
    }
    if names.iter().any(|name| name.contains("vulkan")) {
        backends.push(ComputeBackend::Vulkan);
    }
    if names
        .iter()
        .any(|name| name.contains("hip") || name.contains("rocm"))
    {
        backends.push(ComputeBackend::Rocm);
    }
    if names.iter().any(|name| name.contains("metal")) {
        backends.push(ComputeBackend::Metal);
    }
    backends.push(ComputeBackend::Cpu);
    backends.sort_by_key(|backend| backend.as_str());
    backends.dedup();
    backends
}

fn list_devices(server_path: &Path) -> usize {
    let Ok(output) = Command::new(server_path).arg("--list-devices").output() else {
        return 0;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().filter(|line| line.contains("CUDA")).count()
}

/// Convert a probe result into a capability status.
pub fn status_from_backends(_backends: &[ComputeBackend]) -> CapabilityStatus {
    CapabilityStatus::Supported
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_detection_is_case_insensitive_and_deduped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ggml-cuda.dll"), b"").unwrap();
        std::fs::write(dir.path().join("CUblas64.dll"), b"").unwrap();
        std::fs::write(dir.path().join("vulkan.dll"), b"").unwrap();
        let backends = detect_backends(dir.path());
        assert!(backends.contains(&ComputeBackend::Cuda));
        assert!(backends.contains(&ComputeBackend::Vulkan));
        assert!(backends.contains(&ComputeBackend::Cpu));
        assert_eq!(
            backends
                .iter()
                .filter(|b| **b == ComputeBackend::Cuda)
                .count(),
            1
        );
    }

    #[test]
    fn a_cpu_only_build_is_still_supported() {
        assert_eq!(
            status_from_backends(&[ComputeBackend::Cpu]),
            CapabilityStatus::Supported
        );
    }

    #[test]
    fn server_name_matches_the_platform() {
        if cfg!(windows) {
            assert_eq!(server_name(), "llama-server.exe");
        } else {
            assert_eq!(server_name(), "llama-server");
        }
    }
}
