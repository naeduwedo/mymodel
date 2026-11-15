//! Local IPC for GGUF AI Engine (spec §3.2).
//!
//! # Why a socket and not TCP
//!
//! The core can load models, saturate the GPU and read the model directory, so
//! "who can connect" is a security boundary. A local transport is not reachable
//! from another machine even when a firewall is misconfigured, and its access
//! control is enforced by the OS:
//!
//! | Platform | Transport | Access control |
//! | --- | --- | --- |
//! | Windows | named pipe | DACL: current user + SYSTEM, nobody else |
//! | Linux / macOS | Unix domain socket | 0700 socket directory, peer uid |
//!
//! # Wire format
//!
//! gRPC (HTTP/2) over that transport, using the schema in `proto/`. The first
//! call on a connection should be `HealthService/GetHealth` for liveness; the
//! health reply carries the core version and the capability matrix, so a client
//! can verify protocol compatibility without a second round trip.
//!
//! Only services that are implemented are registered. The hardware, model,
//! runtime and inference services exist in the schema so the contract is
//! frozen, but they are not served until their task lands — a client that
//! calls them gets `UNIMPLEMENTED`, which is the honest answer.

#![warn(missing_docs)]

pub mod client;
pub mod error;
pub mod inference;
pub mod media;
pub mod platform;
pub mod services;
pub mod state;

pub use client::{CoreClient, IPC_CONNECT_TIMEOUT, connect, connect_to};
pub use error::IpcError;
pub use server::{IpcServer, serve, serve_loopback};
pub use state::{CoreState, LifecycleHandle};

pub mod server;

/// Windows-only transport adapter: tonic does not know named pipes, so this
/// module supplies the server-side stream type and the client-side connector.
#[cfg(windows)]
pub mod transport_windows;
