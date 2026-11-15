//! Binding and serving the gateway.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::backend::Backend;
use crate::config::{ApiConfig, ConfigError, Exposure};
use crate::routes::{AppState, router};

/// A running gateway.
#[derive(Debug)]
pub struct ApiServer {
    address: SocketAddr,
    handle: tokio::task::JoinHandle<()>,
}

impl ApiServer {
    /// The address actually bound (useful when port 0 was requested).
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Stop serving.
    pub fn stop(self) {
        self.handle.abort();
    }
}

/// Start the gateway.
///
/// The configuration is validated **before** a socket is opened: a rejected
/// configuration must not leave a listener running that a later request could
/// reach.
pub async fn serve(config: ApiConfig, backend: Arc<dyn Backend>) -> Result<ApiServer, ConfigError> {
    config.validate()?;

    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(|_| ConfigError::TokenRequired { bind: config.bind })?;
    let address = listener.local_addr().unwrap_or(config.bind);

    match config.exposure() {
        Exposure::Loopback => info!(%address, "API gateway listening on loopback"),
        Exposure::Network => warn!(
            %address,
            "API gateway is reachable from the network; every request must carry the bearer token"
        ),
    }

    let state = AppState {
        config: Arc::new(config),
        backend,
    };

    let app = router(state);
    let handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            warn!(error = %err, "the API gateway stopped");
        }
    });

    Ok(ApiServer { address, handle })
}
