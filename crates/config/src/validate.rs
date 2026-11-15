//! Range and cross-field validation (spec §11: schema, defaults, range
//! validation, migration strategy, UI help).

use serde::Serialize;

use crate::{AuthenticationMode, Config, ConfigError, TensorParallelPolicy};

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The configuration cannot be used as written.
    Error,
    /// The configuration works but the user should know the risk.
    Warning,
}

/// A single validation finding, surfaced by the CLI and the Settings page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    /// Field path, e.g. `core.api_port`.
    pub field: String,
    /// Severity.
    pub severity: Severity,
    /// Human readable explanation.
    pub message: String,
}

impl Issue {
    fn error(field: &str, message: impl Into<String>) -> Self {
        Self {
            field: field.to_owned(),
            severity: Severity::Error,
            message: message.into(),
        }
    }

    fn warning(field: &str, message: impl Into<String>) -> Self {
        Self {
            field: field.to_owned(),
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}

/// Returns true when `host` is a loopback literal or `localhost`.
fn is_loopback(host: &str) -> bool {
    let trimmed = host.trim().trim_matches(['[', ']']).to_ascii_lowercase();
    if trimmed == "localhost" {
        return true;
    }
    match trimmed.parse::<std::net::IpAddr>() {
        Ok(addr) => addr.is_loopback(),
        Err(_) => false,
    }
}

/// Collect every finding for a configuration without failing fast, so a UI can
/// show all problems at once.
pub fn check(config: &Config) -> Vec<Issue> {
    let mut issues = Vec::new();

    // --- [core] ---
    if config.core.host.trim().is_empty() {
        issues.push(Issue::error("core.host", "host must not be empty"));
    } else if !is_loopback(&config.core.host) {
        issues.push(Issue::warning(
            "core.host",
            "binding a non-loopback address exposes the core API to the network",
        ));
    }
    if config.core.api_port == 0 {
        issues.push(Issue::error(
            "core.api_port",
            "port 0 is not a valid listen port",
        ));
    }
    if config.core.grpc_port == 0 {
        issues.push(Issue::error(
            "core.grpc_port",
            "port 0 is not a valid listen port",
        ));
    }
    if config.core.api_port == config.core.grpc_port {
        issues.push(Issue::error(
            "core.grpc_port",
            "grpc_port must differ from api_port",
        ));
    }
    if config.core.log_level.trim().is_empty() {
        issues.push(Issue::error(
            "core.log_level",
            "log_level must not be empty",
        ));
    }
    if config.core.health_interval_ms < 100 {
        issues.push(Issue::error(
            "core.health_interval_ms",
            "health_interval_ms must be at least 100",
        ));
    }
    if config.core.heartbeat_timeout_ms <= config.core.health_interval_ms {
        issues.push(Issue::error(
            "core.heartbeat_timeout_ms",
            "heartbeat_timeout_ms must exceed health_interval_ms",
        ));
    }

    // --- [paths] / [downloads].temporary_directory ---
    let path_fields = [
        ("paths.models", config.paths.models.as_str()),
        ("paths.cache", config.paths.cache.as_str()),
        ("paths.runtimes", config.paths.runtimes.as_str()),
        ("paths.logs", config.paths.logs.as_str()),
        ("paths.database", config.paths.database.as_str()),
        (
            "downloads.temporary_directory",
            config.downloads.temporary_directory.as_str(),
        ),
    ];
    for (field, value) in path_fields {
        if value.trim().is_empty() {
            issues.push(Issue::error(field, "path must not be empty"));
        }
    }

    // --- [downloads] ---
    if config.downloads.max_concurrent_downloads == 0 {
        issues.push(Issue::error(
            "downloads.max_concurrent_downloads",
            "at least one concurrent download is required",
        ));
    } else if config.downloads.max_concurrent_downloads > 16 {
        issues.push(Issue::warning(
            "downloads.max_concurrent_downloads",
            "more than 16 concurrent downloads usually hurts throughput and may be rate limited",
        ));
    }
    if config.downloads.max_retries > 20 {
        issues.push(Issue::warning(
            "downloads.max_retries",
            "more than 20 retries may hammer the Hub",
        ));
    }

    // --- [memory] ---
    if config.memory.gpu_reserve_bytes == 0 {
        issues.push(Issue::error(
            "memory.gpu_reserve_bytes",
            "a non-zero VRAM reserve is required to keep the desktop compositor alive",
        ));
    }
    if config.memory.ram_reserve_bytes == 0 {
        issues.push(Issue::error(
            "memory.ram_reserve_bytes",
            "a non-zero RAM reserve is required so the OS is not starved",
        ));
    }
    if config.memory.strategy.trim().is_empty() {
        issues.push(Issue::error(
            "memory.strategy",
            "strategy must not be empty",
        ));
    }

    // --- [gpu] ---
    if config.gpu.strategy.trim().is_empty() {
        issues.push(Issue::error("gpu.strategy", "strategy must not be empty"));
    }
    if config.gpu.allow_tensor_parallel != TensorParallelPolicy::Never
        && !config.gpu.allow_multi_gpu
    {
        issues.push(Issue::error(
            "gpu.allow_tensor_parallel",
            "tensor parallel needs allow_multi_gpu = true",
        ));
    }
    if config.gpu.allow_multi_gpu && !config.runtime.isolate_workers {
        issues.push(Issue::warning(
            "runtime.isolate_workers",
            "multi-GPU plans without isolated workers share an address space with the core",
        ));
    }

    // --- [api] security (spec §9.3) ---
    if config.api.bind.trim().is_empty() {
        issues.push(Issue::error("api.bind", "bind address must not be empty"));
    } else if !is_loopback(&config.api.bind)
        && config.api.authentication == AuthenticationMode::LocalOnly
    {
        issues.push(Issue::error(
            "api.authentication",
            "a non-loopback bind requires authentication = \"token\"",
        ));
    }

    issues
}

/// Validate the configuration, failing when any finding is an error.
pub fn validate(config: &Config) -> Result<Vec<Issue>, ConfigError> {
    let issues = check(config);
    let errors: Vec<&Issue> = issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .collect();
    if errors.is_empty() {
        return Ok(issues);
    }
    let joined = errors
        .iter()
        .map(|issue| format!("{}: {}", issue.field, issue.message))
        .collect::<Vec<_>>()
        .join("; ");
    Err(ConfigError::Invalid(joined))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        validate(&Config::default()).expect("defaults must validate");
    }

    #[test]
    fn loopback_detection() {
        for host in [
            "127.0.0.1",
            "127.0.0.5",
            "::1",
            "[::1]",
            "localhost",
            "LOCALHOST",
        ] {
            assert!(is_loopback(host), "{host} should be loopback");
        }
        for host in ["0.0.0.0", "192.168.1.10", "example.com", ""] {
            assert!(!is_loopback(host), "{host} should not be loopback");
        }
    }

    #[test]
    fn duplicate_ports_are_rejected() {
        let mut cfg = Config::default();
        cfg.core.grpc_port = cfg.core.api_port;
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("grpc_port"), "{err}");
    }

    #[test]
    fn zero_port_is_rejected() {
        let mut cfg = Config::default();
        cfg.core.api_port = 0;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn zero_reserves_are_rejected() {
        let mut cfg = Config::default();
        cfg.memory.gpu_reserve_bytes = 0;
        assert!(validate(&cfg).is_err());

        let mut cfg = Config::default();
        cfg.memory.ram_reserve_bytes = 0;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn lan_bind_requires_token_auth() {
        let mut cfg = Config::default();
        cfg.api.bind = "0.0.0.0".to_owned();
        assert!(
            check(&cfg)
                .iter()
                .any(|i| i.field == "api.authentication" && i.severity == Severity::Error)
        );

        cfg.api.authentication = AuthenticationMode::Token;
        validate(&cfg).expect("token auth makes a LAN bind acceptable");
    }

    #[test]
    fn lan_bind_warns_on_core_host() {
        let mut cfg = Config::default();
        cfg.core.host = "0.0.0.0".to_owned();
        assert!(
            check(&cfg)
                .iter()
                .any(|i| i.field == "core.host" && i.severity == Severity::Warning)
        );
    }

    #[test]
    fn heartbeat_must_exceed_health_interval() {
        let mut cfg = Config::default();
        cfg.core.heartbeat_timeout_ms = cfg.core.health_interval_ms;
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn tensor_parallel_requires_multi_gpu() {
        let mut cfg = Config::default();
        cfg.gpu.allow_multi_gpu = false;
        cfg.gpu.allow_tensor_parallel = TensorParallelPolicy::Always;
        let err = validate(&cfg).unwrap_err();
        assert!(err.to_string().contains("tensor parallel"), "{err}");
    }

    #[test]
    fn empty_path_is_rejected() {
        let mut cfg = Config::default();
        cfg.paths.models = "  ".to_owned();
        assert!(validate(&cfg).is_err());
    }

    #[test]
    fn issues_serialise_for_the_ui() {
        let mut cfg = Config::default();
        cfg.core.api_port = 0;
        let json = serde_json::to_string(&check(&cfg)).unwrap();
        assert!(json.contains("\"severity\":\"error\""), "{json}");
    }
}
