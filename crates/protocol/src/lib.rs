//! Wire protocol for GGUF AI Engine (spec §3.2, §7.2, §8.1, §9).
//!
//! Two layers live here:
//!
//! * **generated Protobuf/gRPC code** — [`pb`], built from
//!   `proto/*.proto` by `build.rs`;
//! * **hand-written value types** — versioning and capability helpers used by
//!   the core and every client, independent of the wire format.
//!
//! Only the services that are actually implemented are registered by the core.
//! `HardwareService`, `ModelService` and friends exist in the schema so the
//! contract is frozen, but their implementations return
//! `CAPABILITY_STATUS_PLANNED`/`UNIMPLEMENTED` until their task lands.

#![warn(missing_docs)]

pub mod convert;
pub mod features;
pub mod snapshot;

/// Generated Protobuf types and gRPC stubs.
pub mod pb {
    #![allow(
        missing_docs,
        clippy::doc_markdown,
        clippy::derive_partial_eq_without_eq
    )]
    tonic::include_proto!("gguf.v1");
}

pub use features::{Feature, FeatureStatus, PIPE_PREFIX, features, ipc_endpoint};

/// Bumped whenever a breaking change is made to the handshake or to an
/// implemented service. Clients must refuse to talk to a core that reports a
/// different major version.
pub const PROTOCOL_VERSION: u32 = 2;
