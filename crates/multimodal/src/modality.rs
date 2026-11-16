//! The modality matrix, derived from probes rather than written by hand.
//!
//! This replaces the earlier hardcoded list. The difference matters: a constant
//! says "vision is supported" whether or not anything is installed, while this
//! says it only when a runtime was found and says why when one was not.

use serde::{Deserialize, Serialize};

use crate::manifest::builtin_manifests;
use crate::probe::{RuntimeProbe, probe_all};

/// Status of one modality on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModalityState {
    /// A runtime that provides it is installed.
    Available,
    /// A runtime exists upstream but is not installed here.
    NotInstalled,
    /// No runtime in this build provides it at all.
    NoRuntime,
}

impl ModalityState {
    /// Stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::NotInstalled => "not_installed",
            Self::NoRuntime => "no_runtime",
        }
    }
}

/// One row of the modality matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModalityReport {
    /// Modality id, e.g. `image_generation`.
    pub modality: String,
    /// Its state here.
    pub state: ModalityState,
    /// Runtime that provides it, when one is installed.
    pub runtime: Option<String>,
    /// Explanation the UI shows verbatim.
    pub detail: String,
}

/// The modalities this build knows about, with their state on this machine.
///
/// The list of names comes from the manifests, so adding a runtime cannot leave a
/// modality stranded, and removing one cannot leave a stale claim.
pub fn modality_report() -> Vec<ModalityReport> {
    let probes = probe_all();
    let manifests = builtin_manifests();

    // Every modality any manifest claims, in a stable order.
    let mut names: Vec<String> = Vec::new();
    for manifest in &manifests {
        for modality in &manifest.modalities {
            if !names.contains(modality) {
                names.push(modality.clone());
            }
        }
    }
    names.sort();

    // Named explicitly so the report answers the question even when no manifest
    // claims the modality, rather than silently omitting it.
    if !names.iter().any(|name| name == "music") {
        names.push("music".to_owned());
    }
    names.sort();

    names
        .into_iter()
        .map(|modality| report_for(&modality, probes))
        .collect()
}

fn report_for(modality: &str, probes: &[RuntimeProbe]) -> ModalityReport {
    let installed = probes
        .iter()
        .find(|probe| probe.installed && probe.manifest.modalities.iter().any(|m| m == modality));

    if let Some(probe) = installed {
        return ModalityReport {
            modality: modality.to_owned(),
            state: ModalityState::Available,
            runtime: Some(probe.manifest.display_name.clone()),
            detail: match &probe.path {
                Some(path) => format!(
                    "provided by {} ({})",
                    probe.manifest.display_name,
                    path.display()
                ),
                None => format!("provided by {}", probe.manifest.display_name),
            },
        };
    }

    // A manifest claims it but the binary is missing: the user can fix this,
    // so the reason must say what to install.
    let expected = probes
        .iter()
        .find(|probe| probe.manifest.modalities.iter().any(|m| m == modality));

    match expected {
        Some(probe) => ModalityReport {
            modality: modality.to_owned(),
            state: ModalityState::NotInstalled,
            runtime: Some(probe.manifest.display_name.clone()),
            detail: probe
                .reason
                .clone()
                .unwrap_or_else(|| format!("{} is not installed", probe.manifest.display_name)),
        },
        None => ModalityReport {
            modality: modality.to_owned(),
            state: ModalityState::NoRuntime,
            runtime: None,
            detail: format!(
                "no runtime in this build provides {modality}; adding one means a new \
                 adapter and a manifest, not a feature flag"
            ),
        },
    }
}

/// True when a runtime for this modality is installed.
pub fn is_available(modality: &str) -> bool {
    modality_report()
        .iter()
        .any(|row| row.modality == modality && row.state == ModalityState::Available)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_covers_every_modality_a_manifest_claims() {
        let report = modality_report();
        for manifest in builtin_manifests() {
            for modality in manifest.modalities {
                assert!(
                    report.iter().any(|row| row.modality == modality),
                    "{modality} is missing from the report"
                );
            }
        }
    }

    #[test]
    fn music_is_always_reported_even_when_no_runtime_provides_it() {
        // Whether music is available depends on audio.cpp being installed, but
        // the row must exist either way: a user asking "can it do music?" gets an
        // answer, not silence.
        let report = modality_report();
        let music = report
            .iter()
            .find(|row| row.modality == "music")
            .expect("music must appear so the question is answered");

        match music.state {
            ModalityState::Available => assert!(music.runtime.is_some()),
            ModalityState::NotInstalled => assert!(!music.detail.is_empty()),
            ModalityState::NoRuntime => assert!(music.detail.contains("no runtime")),
        }
    }

    #[test]
    fn a_modality_without_an_installed_runtime_says_what_to_install() {
        for row in modality_report() {
            if row.state == ModalityState::NotInstalled {
                assert!(
                    !row.detail.is_empty(),
                    "{} must explain itself",
                    row.modality
                );
            }
            if row.state == ModalityState::Available {
                assert!(
                    row.runtime.is_some(),
                    "an available modality names its runtime"
                );
            }
        }
    }

    #[test]
    fn availability_agrees_with_the_report() {
        for row in modality_report() {
            assert_eq!(
                is_available(&row.modality),
                row.state == ModalityState::Available,
                "{} disagrees with itself",
                row.modality
            );
        }
    }
}
