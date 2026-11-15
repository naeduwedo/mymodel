//! Parser errors.
//!
//! Every variant names the *file offset* where possible: a GGUF that fails to
//! parse is a support case, and "invalid metadata at 0x1a2b" is actionable where
//! "parse error" is not.

use thiserror::Error;

/// Why a GGUF file could not be read.
#[derive(Debug, Error)]
pub enum GgufError {
    /// The file could not be opened or mapped.
    #[error("cannot open {path}: {source}")]
    Io {
        /// Path that failed.
        path: String,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },

    /// The first four bytes were not `GGUF`.
    #[error("not a GGUF file: magic is {found:?}, expected \"GGUF\"")]
    BadMagic {
        /// What was found instead.
        found: String,
    },

    /// The file claims a format version this parser does not implement.
    #[error("unsupported GGUF version {found} (this build understands v2 and v3)")]
    UnsupportedVersion {
        /// Version read from the header.
        found: u32,
    },

    /// The file is big-endian. GGUF is defined little-endian; a few historical
    /// files (the "-be" quants) are byte-swapped, and saying so is far more
    /// useful than reporting an absurd version number.
    #[error("this GGUF is big-endian (byte-swapped); only little-endian GGUF is supported")]
    BigEndian,

    /// The file ended before a structure did.
    #[error("unexpected end of file at offset {offset} (needed {needed} more bytes)")]
    Truncated {
        /// Offset where reading stopped making progress.
        offset: u64,
        /// Bytes that were still required.
        needed: u64,
    },

    /// A length or offset would overflow when used.
    #[error("arithmetic overflow at offset {offset}: {what}")]
    Overflow {
        /// Offset of the offending value.
        offset: u64,
        /// What was being computed.
        what: String,
    },

    /// A declared length is larger than any legitimate file.
    #[error(transparent)]
    Limit(#[from] LimitError),

    /// The metadata dictionary contains the same key twice.
    #[error("duplicate metadata key {key:?} at offset {offset}")]
    DuplicateKey {
        /// The repeated key.
        key: String,
        /// Offset where the duplicate was read.
        offset: u64,
    },

    /// A string is not valid UTF-8.
    #[error("invalid UTF-8 in {context} at offset {offset}")]
    InvalidUtf8 {
        /// What was being read (a key, a value, a tensor name).
        context: String,
        /// Offset of the string.
        offset: u64,
    },

    /// A value declared a type this parser does not know.
    #[error("unknown metadata type {value_type} for key {key:?} at offset {offset}")]
    UnknownValueType {
        /// The raw type tag.
        value_type: u32,
        /// Key being read.
        key: String,
        /// Offset of the type tag.
        offset: u64,
    },

    /// A tensor descriptor points outside the file.
    #[error("tensor {name:?} spans {end} bytes but the file is only {file_len} bytes")]
    TensorOutOfBounds {
        /// Tensor name.
        name: String,
        /// Last byte the tensor claims.
        end: u64,
        /// Size of the file.
        file_len: u64,
    },

    /// The alignment field is not a power of two, or is zero.
    #[error("invalid alignment {value} (must be a power of two, at least 1)")]
    BadAlignment {
        /// The value found.
        value: u32,
    },

    /// Tensor data would start before the descriptors ended.
    #[error("tensor data offset {data_offset} precedes the end of the tensor table at {table_end}")]
    BadTensorDataOffset {
        /// Computed start of tensor data.
        data_offset: u64,
        /// End of the tensor table.
        table_end: u64,
    },
}

/// A declared length that exceeds what this parser will accept.
#[derive(Debug, Error)]
#[error("{what} is {value}, which exceeds the limit of {limit}")]
pub struct LimitError {
    /// What was limited.
    pub what: &'static str,
    /// The declared value.
    pub value: u64,
    /// The limit applied.
    pub limit: u64,
}

impl GgufError {
    /// Convenience for a limit violation.
    pub(crate) fn limit(what: &'static str, value: u64, limit: u64) -> Self {
        Self::Limit(LimitError { what, value, limit })
    }
}
