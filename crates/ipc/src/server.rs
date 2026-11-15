//! Binding and serving the local endpoint.
//!
//! The transport is chosen per platform, but the gRPC stack, services and
//! shutdown semantics are identical everywhere:
//!
//! * `bind` creates the endpoint with the platform's access control;
//! * `serve` accepts connections until the shutdown channel fires;
//! * shutdown waits a bounded grace period for in-flight calls, then aborts.

use std::net::SocketAddr;
use std::path::Path;

use gguf_engine_protocol::pb;
use gguf_engine_security::validate_pipe_name;
use tokio::sync::watch;
use tracing::{error, info, warn};

use crate::error::IpcError;
use crate::inference::InferenceService;
use crate::media::MediaService;
use crate::services::{
    EventService, HardwareService, HealthService, ModelService, RuntimeService, UiService,
};
use crate::state::CoreState;

/// How long a graceful shutdown waits for in-flight calls.
///
/// A streaming client (the event stream, and later token streams) can hold a
/// request open indefinitely. The core must still be able to stop — a UI that
/// forgets to disconnect cannot be allowed to pin the process — so shutdown
/// waits this long and then aborts the listener.
pub const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// A running IPC server.
#[derive(Debug)]
pub struct IpcServer {
    endpoint: String,
    shutdown: watch::Sender<bool>,
    handle: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}

impl IpcServer {
    /// Endpoint the server listens on.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Stop accepting connections, then wait [`SHUTDOWN_GRACE`] for in-flight
    /// calls before aborting the listener.
    pub async fn shutdown(self) -> Result<(), IpcError> {
        let _ = self.shutdown.send(true);
        match tokio::time::timeout(SHUTDOWN_GRACE, self.handle).await {
            Ok(Ok(Ok(()))) => {
                info!(endpoint = %self.endpoint, "IPC server stopped");
                Ok(())
            }
            Ok(Ok(Err(err))) => Err(IpcError::Endpoint(err.to_string())),
            Ok(Err(err)) => Err(IpcError::Endpoint(err.to_string())),
            Err(_elapsed) => {
                // Dropping the join handle aborts the accept loop and every
                // connection task it owns.
                warn!(
                    endpoint = %self.endpoint,
                    grace_ms = SHUTDOWN_GRACE.as_millis() as u64,
                    "IPC shutdown exceeded its grace period; connections were aborted"
                );
                Ok(())
            }
        }
    }
}

fn router(state: &CoreState) -> tonic::transport::server::Router {
    tonic::transport::Server::builder()
        .add_service(pb::health_service_server::HealthServiceServer::new(
            HealthService::new(state.clone()),
        ))
        .add_service(pb::event_service_server::EventServiceServer::new(
            EventService::new(state.clone()),
        ))
        .add_service(pb::ui_service_server::UiServiceServer::new(UiService::new(
            state.clone(),
        )))
        .add_service(pb::hardware_service_server::HardwareServiceServer::new(
            HardwareService::new(),
        ))
        .add_service(pb::model_service_server::ModelServiceServer::new(
            ModelService::new(state.clone()),
        ))
        .add_service(pb::runtime_service_server::RuntimeServiceServer::new(
            RuntimeService::new(),
        ))
        .add_service(pb::inference_service_server::InferenceServiceServer::new(
            InferenceService::new(state.clone()),
        ))
        .add_service(pb::media_service_server::MediaServiceServer::new(
            MediaService::new(state),
        ))
}

/// Serve the platform endpoint for a data root.
pub async fn serve(data_root: &Path, state: CoreState) -> Result<IpcServer, IpcError> {
    let endpoint = gguf_engine_protocol::ipc_endpoint(data_root);
    validate_pipe_name(&endpoint)?;
    let (shutdown, mut shutdown_rx) = watch::channel(false);

    #[cfg(windows)]
    {
        use crate::transport_windows::PipeStream;

        // Pipe instances are created through `gguf-engine-security`, which
        // applies a DACL granting the current user and SYSTEM only. tokio's
        // `ServerOptions::create` would fall back to the process default DACL,
        // which on a desktop typically includes the local Users group.
        let first = gguf_engine_security::windows_pipe::create(&endpoint).map_err(|err| {
            IpcError::Bind {
                endpoint: endpoint.clone(),
                source: std::io::Error::other(err.to_string()),
            }
        })?;

        let accept_endpoint = endpoint.clone();
        // A named pipe accepts exactly one client per instance, so the
        // endpoint must always have exactly one instance listening, and its
        // replacement must be created immediately after a client is accepted.
        //
        // Both alternatives look like "the core is not running" to a client:
        //   * creating the replacement only after the connection is served
        //     leaves a window with nothing listening;
        //   * keeping several instances listening at once lets a client attach
        //     to an instance nobody is accepting on, after which the accept
        //     loop waits forever on the original.
        let incoming = async_stream::stream! {
            let mut listening = first;
            loop {
                let server = match listening.connect().await {
                    Ok(server) => server,
                    Err(err) => {
                        warn!(error = %err, "pipe client cancelled");
                        match gguf_engine_security::windows_pipe::create(&accept_endpoint) {
                            Ok(fresh) => {
                                listening = fresh;
                                continue;
                            }
                            Err(err) => {
                                warn!(error = %err, "creating a pipe instance failed");
                                return;
                            }
                        }
                    }
                };

                // Replace the instance *before* yielding, so the endpoint is
                // listening again by the time the next client looks for it.
                match gguf_engine_security::windows_pipe::create(&accept_endpoint) {
                    Ok(next) => listening = next,
                    Err(err) => {
                        warn!(error = %err, "creating a pipe instance failed");
                        return;
                    }
                }

                yield Ok::<PipeStream, std::io::Error>(PipeStream::new(server));
            }
        };

        info!(
            %endpoint,
            dacl = %gguf_engine_security::pipe_acl_sddl().unwrap_or_default(),
            "IPC listening on a named pipe (DACL: current user + SYSTEM)"
        );
        let handle = tokio::spawn(async move {
            router(&state)
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = shutdown_rx.changed().await;
                })
                .await
        });
        Ok(IpcServer {
            endpoint,
            shutdown,
            handle,
        })
    }

    #[cfg(unix)]
    {
        let socket_dir = gguf_engine_security::socket_dir(data_root);
        gguf_engine_security::secure_socket_directory(&socket_dir)?;
        let socket_path = socket_dir.join("core.sock");
        // The single-instance lock guarantees no other core owns this path, so a
        // leftover file is a crashed predecessor's socket.
        if socket_path.exists() {
            std::fs::remove_file(&socket_path).map_err(|source| IpcError::Bind {
                endpoint: socket_path.display().to_string(),
                source,
            })?;
        }
        let listener =
            tokio::net::UnixListener::bind(&socket_path).map_err(|source| IpcError::Bind {
                endpoint: socket_path.display().to_string(),
                source,
            })?;
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);

        info!(path = %socket_path.display(), "IPC listening on a Unix socket (0700 directory)");
        let handle = tokio::spawn(async move {
            router(&state)
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = shutdown_rx.changed().await;
                })
                .await
        });
        Ok(IpcServer {
            endpoint: socket_path.display().to_string(),
            shutdown,
            handle,
        })
    }

    #[cfg(not(any(windows, unix)))]
    {
        let _ = (endpoint, state, shutdown_rx, shutdown);
        Err(IpcError::Endpoint(
            "this platform has no local IPC transport".into(),
        ))
    }
}

/// Serve on a loopback TCP port.
///
/// Not used in production: it exists so integration tests and future remote
/// workers can exercise the same services without platform-specific sockets.
/// It binds loopback only, never `0.0.0.0`.
pub async fn serve_loopback(state: CoreState) -> Result<IpcServer, IpcError> {
    let incoming =
        tonic::transport::server::TcpIncoming::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .map_err(|err| IpcError::Endpoint(err.to_string()))?
            .with_nodelay(Some(true));
    let local = incoming
        .local_addr()
        .map_err(|err| IpcError::Endpoint(err.to_string()))?;
    let (shutdown, mut shutdown_rx) = watch::channel(false);

    info!(%local, "IPC listening on loopback TCP (test transport)");
    let handle = tokio::spawn(async move {
        router(&state)
            .serve_with_incoming_shutdown(incoming, async move {
                let _ = shutdown_rx.changed().await;
            })
            .await
    });

    Ok(IpcServer {
        endpoint: local.to_string(),
        shutdown,
        handle,
    })
}

/// Human-readable hint for a failed bind.
pub fn describe_bind_failure(endpoint: &str, error: &IpcError) -> String {
    match error {
        IpcError::Bind { .. } => {
            format!("could not bind {endpoint}: {error}. Another core may still be shutting down.")
        }
        other => {
            error!(%other, "IPC bind failed");
            other.to_string()
        }
    }
}
