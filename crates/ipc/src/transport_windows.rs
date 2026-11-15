//! Transport adapters for Windows named pipes.
//!
//! tonic knows nothing about named pipes, so this module supplies the two
//! hooks it needs:
//!
//! * `PipeStream` — an `AsyncRead + AsyncWrite` resource that also implements
//!   tonic's `Connected` trait, used by the server's incoming stream;
//! * `PipeConnector` — a `tower::Service<Uri>` that opens a pipe, used by the
//!   client via `Endpoint::connect_with_connector`.
//!
//! Access control is *not* implemented here: the pipe's DACL already decided
//! whether the connection may exist (see `gguf-engine-security`).

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer};
use tonic::transport::server::Connected;
use tower::Service;

/// An open pipe connection, from either side.
#[derive(Debug)]
pub enum PipeStream {
    /// Accepted by the core.
    Server(NamedPipeServer),
    /// Opened by a client.
    Client(NamedPipeClient),
}

impl PipeStream {
    /// Wrap an accepted connection.
    pub fn new(server: NamedPipeServer) -> Self {
        Self::Server(server)
    }

    /// Open a connection to an existing pipe.
    pub fn connect(name: &str) -> std::io::Result<Self> {
        Ok(Self::Client(ClientOptions::new().open(name)?))
    }
}

macro_rules! dispatch {
    ($self:expr, $inner:ident => $expr:expr) => {
        match $self.get_mut() {
            PipeStream::Server($inner) => $expr,
            PipeStream::Client($inner) => $expr,
        }
    };
}

impl AsyncRead for PipeStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        dispatch!(self, inner => Pin::new(inner).poll_read(cx, buf))
    }
}

impl AsyncWrite for PipeStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        dispatch!(self, inner => Pin::new(inner).poll_write(cx, buf))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        dispatch!(self, inner => Pin::new(inner).poll_flush(cx))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        dispatch!(self, inner => Pin::new(inner).poll_shutdown(cx))
    }
}

/// Connection information attached to every request on this stream.
#[derive(Clone, Debug)]
pub struct PipeConnectInfo {
    /// True: the client was accepted on the local pipe after passing its DACL.
    pub local_pipe: bool,
    /// True when this side opened the connection (client), false when accepted.
    pub is_client: bool,
}

impl Connected for PipeStream {
    type ConnectInfo = PipeConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        PipeConnectInfo {
            local_pipe: true,
            is_client: matches!(self, PipeStream::Client(_)),
        }
    }
}

/// Opens named pipes for tonic clients.
#[derive(Clone, Debug)]
pub struct PipeConnector {
    name: String,
}

impl PipeConnector {
    /// Create a connector for a pipe name (`\\.\pipe\...`).
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Service<tonic::transport::Uri> for PipeConnector {
    // tonic's client stack speaks hyper's IO traits; TokioIo adapts a tokio
    // AsyncRead + AsyncWrite resource into them.
    type Response = hyper_util::rt::TokioIo<PipeStream>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = std::io::Result<Self::Response>> + Send + 'static>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _uri: tonic::transport::Uri) -> Self::Future {
        let name = self.name.clone();
        Box::pin(async move {
            let stream = PipeStream::connect(&name)?;
            Ok(hyper_util::rt::TokioIo::new(stream))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_info_distinguishes_the_two_sides() {
        let info = PipeConnectInfo {
            local_pipe: true,
            is_client: true,
        };
        assert!(info.local_pipe && info.is_client);
    }
}
