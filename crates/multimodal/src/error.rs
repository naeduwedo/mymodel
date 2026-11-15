//! Errors shared by the multimodal adapters.

/// Why a multimodal request failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MultimodalError {
    /// The request itself was wrong.
    #[error("{0}")]
    Invalid(String),

    /// No runtime provides this modality on this machine.
    #[error("{0}")]
    NotAvailable(String),

    /// The runtime was found but the call failed.
    #[error("{0}")]
    Runtime(String),
}
