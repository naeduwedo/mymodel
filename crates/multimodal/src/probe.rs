//! Locating and probing the non-LLM runtimes.
//!
//! A probe answers one question: is this runtime installed **and usable here**?
//! It never guesses. A binary that is missing is reported missing with the
//! directory that was searched, because "not installed" is a fact the user can
//! act on and "unsupported" is not.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::manifest::{Invocation, RuntimeManifest, builtin_manifests};

/// Environment variable that points at a runtime's directory.
pub const RUNTIME_ENV: &str = "GGUF_ENGINE_RUNTIME_DIR";

/// The outcome of probing one runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProbe {
    /// Its manifest.
    pub manifest: RuntimeManifest,
    /// Whether a usable binary was found.
    pub installed: bool,
    /// Where it was found.
    pub path: Option<PathBuf>,
    /// Why it is not usable, when it is not.
    pub reason: Option<String>,
}

impl RuntimeProbe {
    /// A runtime that could not be found.
    fn missing(manifest: RuntimeManifest, reason: String) -> Self {
        Self {
            manifest,
            installed: false,
            path: None,
            reason: Some(reason),
        }
    }
}

/// Directories searched, in order, for every runtime binary.
///
/// Explicit configuration first, then the conventional install roots. The list
/// is short on purpose: a binary found by accident in an unrelated directory is
/// more likely to be a different build than the one the user meant.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(value) = std::env::var(RUNTIME_ENV) {
        for part in value.split(';') {
            if !part.trim().is_empty() {
                dirs.push(PathBuf::from(part.trim()));
            }
        }
    }

    for key in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Ok(root) = std::env::var(key) {
            dirs.push(PathBuf::from(root).join("gguf-ai-engine").join("runtimes"));
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/gguf-ai-engine/runtimes"));
    }

    dirs
}

/// The executable file name for a runtime binary on this platform.
pub fn binary_file_name(binary: &str) -> String {
    if cfg!(windows) {
        format!("{binary}.exe")
    } else {
        binary.to_owned()
    }
}

/// Find a runtime binary, first on `PATH`, then in [`search_dirs`].
///
/// `PATH` is consulted because a user who installed the runtime themselves
/// expects it to work without also telling this program where it went.
pub fn locate(binary: &str) -> Option<PathBuf> {
    let file = binary_file_name(binary);

    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(&file);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    for dir in search_dirs() {
        let candidate = dir.join(&file);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

/// Probe one manifest.
pub fn probe(manifest: RuntimeManifest) -> RuntimeProbe {
    if !manifest.runs_here() {
        return RuntimeProbe::missing(
            manifest,
            format!(
                "this runtime does not support {} on {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        );
    }

    let binary = manifest.binary().to_owned();
    match locate(&binary) {
        Some(path) => RuntimeProbe {
            manifest,
            installed: true,
            path: Some(path),
            reason: None,
        },
        None => {
            let searched = search_dirs()
                .iter()
                .map(|dir| dir.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            RuntimeProbe::missing(
                manifest,
                format!(
                    "{} was not found on PATH or in: {searched}. Set {} to its directory.",
                    binary_file_name(&binary),
                    RUNTIME_ENV
                ),
            )
        }
    }
}

/// Probe every known runtime once, caching the result for the process.
///
/// Probing touches the filesystem, and the modality registry is read on every
/// status request; caching keeps that off the hot path without making the answer
/// stale for the life of a request.
pub fn probe_all() -> &'static [RuntimeProbe] {
    static CACHE: OnceLock<Vec<RuntimeProbe>> = OnceLock::new();
    CACHE.get_or_init(|| builtin_manifests().into_iter().map(probe).collect())
}

/// The first installed runtime that provides `modality`.
pub fn runtime_for(modality: &str) -> Option<&'static RuntimeProbe> {
    probe_all().iter().find(|probe| {
        probe.installed
            && probe
                .manifest
                .modalities
                .iter()
                .any(|value| value == modality)
    })
}

/// The HTTP base URL for an installed runtime, when it is invoked over HTTP.
pub fn http_base(probe: &RuntimeProbe) -> Option<String> {
    match &probe.manifest.invocation {
        Invocation::Http { default_port, .. } => Some(format!("http://127.0.0.1:{default_port}")),
        Invocation::Cli { .. } => None,
    }
}

/// Whether a path is an executable this process may run.
pub fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runtime_that_is_not_installed_says_where_it_looked() {
        let mut manifest = builtin_manifests().remove(0);
        manifest.invocation = Invocation::Cli {
            binary: "definitely-not-installed-xyzzy".to_owned(),
        };

        let probe = probe(manifest);
        assert!(!probe.installed);
        let reason = probe.reason.expect("a reason");
        assert!(reason.contains("not found"), "{reason}");
        assert!(
            reason.contains(RUNTIME_ENV),
            "it must say how to fix it: {reason}"
        );
    }

    #[test]
    fn a_runtime_for_another_platform_is_refused_without_searching() {
        let mut manifest = builtin_manifests().remove(0);
        manifest.os = vec!["plan9".to_owned()];

        let probe = probe(manifest);
        assert!(!probe.installed);
        assert!(
            probe.reason.unwrap().contains("does not support"),
            "the refusal must name the platform"
        );
    }

    #[test]
    fn probing_every_runtime_is_stable_and_cached() {
        let first = probe_all().as_ptr();
        let second = probe_all().as_ptr();
        assert_eq!(first, second, "the probe result must be cached");
        assert!(!probe_all().is_empty());
    }

    #[test]
    fn a_missing_modality_has_no_runtime() {
        // Nothing implements music generation, so this must be None even on a
        // machine with every other runtime installed.
        assert!(runtime_for("music").is_none());
    }

    #[test]
    fn only_http_runtimes_have_a_base_url() {
        for probe in probe_all() {
            match &probe.manifest.invocation {
                Invocation::Http { .. } => {
                    assert!(http_base(probe).is_some());
                }
                Invocation::Cli { .. } => {
                    assert!(http_base(probe).is_none());
                }
            }
        }
    }
}
