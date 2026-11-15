//! Configuration error types.

use thiserror::Error;

/// Errors raised while loading or validating configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("failed to read configuration file {path}: {source}")]
    Io {
        /// Path that failed to read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The TOML document is malformed or contains unknown keys.
    #[error("failed to parse configuration: {0}")]
    Parse(#[from] toml::de::Error),

    /// The document could not be serialised.
    #[error("failed to serialise configuration: {0}")]
    Serialize(#[from] toml::ser::Error),

    /// One or more values violate the schema.
    #[error("invalid configuration: {0}")]
    Invalid(String),

    /// A configured path escapes its permitted root.
    #[error("path {path} escapes the application data root {root}")]
    PathEscape {
        /// Offending path.
        path: String,
        /// Root the path must stay inside.
        root: String,
    },

    /// The platform data directory could not be determined.
    #[error("could not determine the platform application data directory: {0}")]
    NoDataDirectory(String),
}
