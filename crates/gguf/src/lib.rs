//! GGUF container parsing (spec §7.1).
//!
//! # What this crate promises
//!
//! * **Read-only.** No code path writes to a model file.
//! * **Bounded.** Every length taken from the file is checked against
//!   [`reader::Limits`] before it sizes an allocation, and every read goes through
//!   a cursor that cannot leave the mapped region.
//! * **Mapped, not loaded.** A 100 GB model is memory-mapped; only the pages the
//!   parser touches are faulted in.
//! * **Honest about the unknown.** A tensor type this build does not know keeps
//!   its numeric id and is reported as unknown rather than being called "F32" or
//!   silently dropped.
//!
//! # What it deliberately does not do
//!
//! * No tensor execution, no dequantisation, no shape inference beyond the file.
//! * No repair: a malformed file is rejected with a location, never "fixed".

#![warn(missing_docs)]

pub mod error;
pub mod manifest;
pub mod parser;
pub mod reader;
pub mod value;

pub use error::{GgufError, LimitError};
pub use manifest::{
    CompatibilityReport, CompatibilityState, KNOWN_ARCHITECTURES, MANIFEST_SCHEMA_VERSION,
    Modality, ModelFile, ModelManifest, RuntimeRequirement, compatibility, manifest_from_gguf,
};
pub use parser::{
    Gguf, HEADER_READ_LIMIT, TensorInfo, align_up, parse_bytes, parse_file, parse_prefix,
    parse_with_limits,
};
pub use reader::Limits;
pub use value::{KnownTensorType, TensorType, Value};
