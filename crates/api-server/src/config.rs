//! Gateway configuration and the security rules it enforces (spec §9.2).

use std::net::{IpAddr, SocketAddr};

/// Default loopback port. Chosen to avoid the common 8080/3000 collisions.
pub const DEFAULT_PORT: u16 = 8720;

/// How the gateway is allowed to be reachable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposure {
    /// Loopback only. The default, and the only mode that needs no token.
    Loopback,
    /// Reachable from the network. Requires a bearer token.
    Network,
}

/// Gateway settings.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// Address to bind.
    pub bind: SocketAddr,
    /// Bearer token required on every request. Mandatory for non-loopback binds.
    pub token: Option<String>,
    /// Whether cross-origin browser requests are allowed.
    pub allow_browser_origins: bool,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], DEFAULT_PORT)),
            token: None,
            allow_browser_origins: false,
        }
    }
}

/// Why a configuration was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// A non-loopback bind was requested without a token.
    #[error(
        "binding {bind} exposes the API beyond this machine; set an API token first \
         (spec §9.2 requires a bearer token for any non-loopback bind)"
    )]
    TokenRequired {
        /// The address that was requested.
        bind: SocketAddr,
    },

    /// The token is present but too weak to be meaningful.
    #[error("the API token must be at least {minimum} characters")]
    TokenTooShort {
        /// Minimum length.
        minimum: usize,
    },
}

impl ApiConfig {
    /// How this configuration exposes the gateway.
    pub fn exposure(&self) -> Exposure {
        if self.bind.ip().is_loopback() {
            Exposure::Loopback
        } else {
            Exposure::Network
        }
    }

    /// Check the configuration before a socket is opened.
    ///
    /// The rule that matters: a non-loopback bind without a token would put an
    /// unauthenticated model endpoint on the network, so it is refused outright
    /// rather than warned about.
    pub fn validate(&self) -> Result<(), ConfigError> {
        match (&self.token, self.exposure()) {
            (None, Exposure::Network) => Err(ConfigError::TokenRequired { bind: self.bind }),
            (Some(token), _) if token.len() < MIN_TOKEN_LEN => Err(ConfigError::TokenTooShort {
                minimum: MIN_TOKEN_LEN,
            }),
            _ => Ok(()),
        }
    }

    /// Whether a request carrying `presented` may proceed.
    pub fn authorise(&self, presented: Option<&str>) -> bool {
        match &self.token {
            None => self.exposure() == Exposure::Loopback,
            Some(expected) => presented.is_some_and(|value| constant_time_eq(value, expected)),
        }
    }
}

/// Shortest token that is not trivially guessable.
pub const MIN_TOKEN_LEN: usize = 16;

/// Compare without leaking the length of the matching prefix.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Whether an address is loopback, for callers building a config.
pub fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}
