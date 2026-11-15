//! Client side of the local IPC transport.
//!
//! Used by `gguf doctor`, the daemon's self-check and (from Task 3) the native
//! UIs. The client deliberately connects lazily and maps "nothing is listening"
//! to a distinct error so callers can print "the core is not running" instead
//! of a connection trace.

use std::time::Duration;

use gguf_engine_protocol::pb;
use tonic::transport::{Channel, Endpoint};
use tracing::debug;

use crate::error::IpcError;
use crate::platform;

/// How long a connect may take before it is reported as "core not running".
pub const IPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A connected core.
#[derive(Debug, Clone)]
pub struct CoreClient {
    channel: Channel,
    endpoint: String,
}

impl CoreClient {
    /// Endpoint this client talks to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Health and capability report.
    pub async fn health(&self) -> Result<pb::HealthReport, IpcError> {
        let mut client = pb::health_service_client::HealthServiceClient::new(self.channel.clone());
        let reply = client.get_health(pb::HealthRequest {}).await?;
        Ok(reply.into_inner())
    }

    /// Cheap liveness probe.
    pub async fn ping(&self) -> Result<(), IpcError> {
        let mut client = pb::health_service_client::HealthServiceClient::new(self.channel.clone());
        client.ping(pb::Empty {}).await?;
        Ok(())
    }

    /// Presentation snapshot for the native UI shells (Overview + Settings).
    pub async fn snapshot(&self) -> Result<pb::UiSnapshot, IpcError> {
        let mut client = pb::ui_service_client::UiServiceClient::new(self.channel.clone());
        let reply = client.get_snapshot(pb::UiSnapshotRequest {}).await?;
        Ok(reply.into_inner())
    }

    /// Hardware inventory (CPU, memory, storage, GPUs).
    pub async fn hardware(&self) -> Result<pb::HardwareReport, IpcError> {
        let mut client =
            pb::hardware_service_client::HardwareServiceClient::new(self.channel.clone());
        let reply = client.get_hardware(pb::HardwareRequest {}).await?;
        Ok(reply.into_inner())
    }

    /// Run a chat completion, streaming tokens as they are generated.
    ///
    /// The stream ends with a `done` chunk carrying the token count and elapsed
    /// time, or with an error. Dropping the returned stream cancels the request:
    /// the core notices the disconnect and unloads the model.
    pub async fn chat(
        &self,
        request: pb::ChatRequest,
    ) -> Result<tonic::Streaming<pb::ChatChunk>, IpcError> {
        let mut client =
            pb::inference_service_client::InferenceServiceClient::new(self.channel.clone());
        let reply = client.chat(request).await?;
        Ok(reply.into_inner())
    }

    /// Stream events after `after_seq`, replaying from the core's ring first.
    pub async fn subscribe(&self, after_seq: u64) -> Result<tonic::Streaming<pb::Event>, IpcError> {
        let mut client = pb::event_service_client::EventServiceClient::new(self.channel.clone());
        let reply = client.subscribe(pb::EventRequest { after_seq }).await?;
        Ok(reply.into_inner())
    }
}

/// Connect to the endpoint for a data root, reporting "not running" clearly.
///
/// The probe and the connection are the *same* attempt: on Windows every extra
/// connection consumes the single listening pipe instance, so an "is anything
/// there?" check would disturb the connection that follows it.
pub async fn connect(data_root: &std::path::Path) -> Result<CoreClient, IpcError> {
    let endpoint = platform::endpoint_for(data_root);
    match connect_to(&endpoint).await {
        Ok(client) => Ok(client),
        // Any failure to reach a local endpoint means the core is not serving:
        // there is nothing else that could be listening there.
        Err(IpcError::Endpoint(_)) | Err(IpcError::Transport(_)) => {
            Err(IpcError::NotRunning(endpoint))
        }
        Err(other) => Err(other),
    }
}

/// Connect to an explicit endpoint (socket path, pipe name or `host:port`).
pub async fn connect_to(endpoint: &str) -> Result<CoreClient, IpcError> {
    let channel = connect_channel(endpoint).await?;
    Ok(CoreClient {
        channel,
        endpoint: endpoint.to_owned(),
    })
}

#[cfg(windows)]
async fn connect_channel(endpoint: &str) -> Result<Channel, IpcError> {
    use crate::transport_windows::PipeConnector;

    if !endpoint.starts_with("\\\\") {
        return connect_tcp(endpoint).await;
    }

    // tonic has no pipe support, so the channel is built on a custom connector.
    // The URI is a placeholder: the connector opens `endpoint` and ignores it.
    let uri = Endpoint::from_static("http://localhost/_pipe");
    debug!(%endpoint, "connecting over a named pipe");
    uri.connect_timeout(IPC_CONNECT_TIMEOUT)
        .connect_with_connector(PipeConnector::new(endpoint))
        .await
        .map_err(|err| IpcError::Endpoint(format!("{endpoint}: {err}")))
}

#[cfg(unix)]
async fn connect_channel(endpoint: &str) -> Result<Channel, IpcError> {
    if endpoint.starts_with('/') {
        let uri =
            Endpoint::from_static("http://localhost/_sock").connect_timeout(IPC_CONNECT_TIMEOUT);
        debug!(%endpoint, "connecting over a Unix socket");
        return uri
            .connect_with_connector(tower::service_fn(move |_| {
                let path = endpoint.to_owned();
                async move {
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(
                        tokio::net::UnixStream::connect(path).await?,
                    ))
                }
            }))
            .await
            .map_err(|err| IpcError::Endpoint(format!("{endpoint}: {err}")));
    }
    connect_tcp(endpoint).await
}

#[cfg(not(any(windows, unix)))]
async fn connect_channel(endpoint: &str) -> Result<Channel, IpcError> {
    connect_tcp(endpoint).await
}

async fn connect_tcp(endpoint: &str) -> Result<Channel, IpcError> {
    let uri = uri_for(endpoint)?;
    debug!(%uri, "connecting over TCP");
    Endpoint::from_shared(uri)
        .map_err(|err| IpcError::Endpoint(err.to_string()))?
        .connect_timeout(IPC_CONNECT_TIMEOUT)
        .tcp_nodelay(true)
        .connect()
        .await
        .map_err(|err| IpcError::Endpoint(format!("{endpoint}: {err}")))
}

/// Reward the endpoint as a transport URI.
///
/// * a branch pipe name becomes `http://localhost/_pipe/<name>` (hyper ignores
///   the path for a custom connector, but it must be present);
/// * an absolute Unix path becomes `http://localhost/_sock/<path>`;
/// * anything containing `:` is treated as `host:port` for the loopback test
///   transport.
fn uri_for(endpoint: &str) -> Result<String, IpcError> {
    if endpoint.starts_with("\\\\") {
        let name = endpoint.rsplit('\\').next().unwrap_or(endpoint);
        return Ok(format!("http://localhost/_pipe/{name}"));
    }
    if endpoint.contains(':') && !endpoint.starts_with('/') {
        return Ok(format!("http://{endpoint}"));
    }
    if endpoint.is_empty() {
        return Err(IpcError::Endpoint("empty IPC endpoint".into()));
    }
    Ok(format!("http://localhost/_sock/{endpoint}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_names_become_pipe_uris() {
        let uri = uri_for(r"\\?\pipe\gguf-ai-engine-abc").unwrap();
        assert!(uri.starts_with("http://localhost/_pipe/"), "{uri}");
    }

    #[test]
    fn socket_paths_become_sock_uris() {
        let uri = uri_for("/tmp/gguf/run/core.sock").unwrap();
        assert_eq!(uri, "http://localhost/_sock//tmp/gguf/run/core.sock");
    }

    #[test]
    fn host_port_is_passed_through() {
        assert_eq!(uri_for("127.0.0.1:5555").unwrap(), "http://127.0.0.1:5555");
    }

    #[test]
    fn empty_endpoint_is_rejected() {
        assert!(matches!(uri_for(""), Err(IpcError::Endpoint(_))));
    }
}
