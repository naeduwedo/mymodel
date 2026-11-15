//! Typed configuration for GGUF AI Engine.
//!
//! Implements spec §11 (configuration file) and §9.3 (secure defaults):
//!
//! * every field has an explicit schema, default value and range validation;
//! * paths are normalised and resolved against a single application data root;
//! * parsing is strict — unknown keys are rejected so an outdated binary can
//!   never silently ignore a policy the user believes is active.

#![warn(missing_docs)]

mod error;
pub mod paths;
mod validate;

pub use error::ConfigError;
pub use paths::{AppPaths, Origin};
pub use validate::{Issue, Severity, check, validate};

use serde::{Deserialize, Serialize};

/// Filename used when a configuration file is written next to the others.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Root configuration document.
///
/// Mirrors `config.toml` from the specification. Sections are optional on
/// disk: a missing section falls back to the section default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Core daemon settings.
    #[serde(default)]
    pub core: CoreConfig,
    /// Filesystem locations relative to the application data root.
    #[serde(default)]
    pub paths: PathsConfig,
    /// Download manager settings.
    #[serde(default)]
    pub downloads: DownloadsConfig,
    /// Memory orchestrator settings.
    #[serde(default)]
    pub memory: MemoryConfig,
    /// GPU scheduling settings.
    #[serde(default)]
    pub gpu: GpuConfig,
    /// Runtime manager settings.
    #[serde(default)]
    pub runtime: RuntimeConfig,
    /// Local API server settings.
    #[serde(default)]
    pub api: ApiConfig,
    /// Non-LLM runtime settings (spec §2.3).
    #[serde(default)]
    pub multimodal: MultimodalConfig,
}

/// `[multimodal]` — model files for the image, speech and audio runtimes.
///
/// The runtime binaries are discovered from `PATH` and
/// `GGUF_ENGINE_RUNTIME_DIR`; what cannot be guessed is which model file to load,
/// so that is configuration. An empty entry means "this modality is not set up",
/// which the probe reports as such rather than failing at request time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultimodalConfig {
    /// Diffusion model for image and video generation (a `.gguf` or `.safetensors`).
    #[serde(default)]
    pub image_model: Option<String>,
    /// URL of a running diffusion server, e.g. `http://127.0.0.1:1234`.
    ///
    /// The runtime is its own process: it needs the model on its command line and
    /// preloading it takes tens of seconds, so the engine points at one rather
    /// than starting a fresh server per request. Empty means "use the runtime's
    /// default port".
    #[serde(default)]
    pub diffusion_url: Option<String>,
    /// whisper.cpp model file (`ggml-*.bin`).
    #[serde(default)]
    pub asr_model: Option<String>,
    /// Piper voice model (`*.onnx`).
    #[serde(default)]
    pub tts_voice: Option<String>,
}

/// `[core]` — daemon identity, ports and lifecycle behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreConfig {
    /// Interface the REST API binds to. Defaults to loopback (spec §9.3).
    #[serde(default = "CoreConfig::default_host")]
    pub host: String,
    /// Loopback REST API port.
    #[serde(default = "CoreConfig::default_api_port")]
    pub api_port: u16,
    /// Loopback gRPC port used by the native UIs.
    #[serde(default = "CoreConfig::default_grpc_port")]
    pub grpc_port: u16,
    /// `tracing` filter directive for stdout/file logs.
    #[serde(default = "CoreConfig::default_log_level")]
    pub log_level: String,
    /// Register the core as a login item so the desktop app can start with the
    /// user session. Off by default: nothing is installed into the OS login
    /// items without the user asking for it.
    #[serde(default = "default_false")]
    pub start_with_login: bool,
    /// Keep workers alive when the last UI disconnects (spec §3.1).
    #[serde(default = "default_true")]
    pub keep_workers_after_ui_exit: bool,
    /// Period between worker health probes, in milliseconds.
    #[serde(default = "CoreConfig::default_health_interval_ms")]
    pub health_interval_ms: u64,
    /// How long a worker may miss heartbeats before it is considered dead.
    #[serde(default = "CoreConfig::default_heartbeat_timeout_ms")]
    pub heartbeat_timeout_ms: u64,
}

impl CoreConfig {
    fn default_host() -> String {
        "127.0.0.1".to_owned()
    }
    fn default_api_port() -> u16 {
        18888
    }
    fn default_grpc_port() -> u16 {
        18889
    }
    fn default_log_level() -> String {
        "info".to_owned()
    }
    fn default_health_interval_ms() -> u64 {
        5_000
    }
    fn default_heartbeat_timeout_ms() -> u64 {
        30_000
    }
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            host: Self::default_host(),
            api_port: Self::default_api_port(),
            grpc_port: Self::default_grpc_port(),
            log_level: Self::default_log_level(),
            start_with_login: false,
            keep_workers_after_ui_exit: true,
            health_interval_ms: Self::default_health_interval_ms(),
            heartbeat_timeout_ms: Self::default_heartbeat_timeout_ms(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

/// `[paths]` — directories, all relative to the application data root unless
/// absolute. Resolution and containment checks live in [`AppPaths`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathsConfig {
    /// Downloaded and imported model files.
    #[serde(default = "PathsConfig::default_models")]
    pub models: String,
    /// Scratch space for staging buffers and the runtime cache.
    #[serde(default = "PathsConfig::default_cache")]
    pub cache: String,
    /// Installed runtime binaries.
    #[serde(default = "PathsConfig::default_runtimes")]
    pub runtimes: String,
    /// Log files.
    #[serde(default = "PathsConfig::default_logs")]
    pub logs: String,
    /// SQLite database directory.
    #[serde(default = "PathsConfig::default_database")]
    pub database: String,
}

impl PathsConfig {
    fn default_models() -> String {
        "models".to_owned()
    }
    fn default_cache() -> String {
        "cache".to_owned()
    }
    fn default_runtimes() -> String {
        "runtimes".to_owned()
    }
    fn default_logs() -> String {
        "logs".to_owned()
    }
    fn default_database() -> String {
        "database".to_owned()
    }
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            models: Self::default_models(),
            cache: Self::default_cache(),
            runtimes: Self::default_runtimes(),
            logs: Self::default_logs(),
            database: Self::default_database(),
        }
    }
}

/// `[downloads]` — Hugging Face download behaviour (spec §7.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadsConfig {
    /// Maximum number of files transferred concurrently.
    #[serde(default = "DownloadsConfig::default_max_concurrent")]
    pub max_concurrent_downloads: u32,
    /// Allow HTTP Range resume of partial files.
    #[serde(default = "default_true")]
    pub resume: bool,
    /// Verify SHA-256 (and ETag where available) before marking files ready.
    #[serde(default = "default_true")]
    pub verify_hashes: bool,
    /// Directory where partial downloads are staged before an atomic rename.
    #[serde(default = "DownloadsConfig::default_temp_dir")]
    pub temporary_directory: String,
    /// Retry attempts for a failed chunk/request.
    #[serde(default = "DownloadsConfig::default_retries")]
    pub max_retries: u32,
}

impl DownloadsConfig {
    fn default_max_concurrent() -> u32 {
        3
    }
    fn default_temp_dir() -> String {
        "cache/downloads".to_owned()
    }
    fn default_retries() -> u32 {
        5
    }
}

impl Default for DownloadsConfig {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: Self::default_max_concurrent(),
            resume: true,
            verify_hashes: true,
            temporary_directory: Self::default_temp_dir(),
            max_retries: Self::default_retries(),
        }
    }
}

/// `[memory]` — resource reserves enforced by the memory orchestrator (spec §6.5).
///
/// All capacities are integer bytes; floating-point gigabytes are never
/// persisted (spec §10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    /// Planner strategy name.
    #[serde(default = "MemoryConfig::default_strategy")]
    pub strategy: String,
    /// VRAM kept free on every GPU for the engine itself.
    #[serde(default = "MemoryConfig::default_gpu_reserve")]
    pub gpu_reserve_bytes: u64,
    /// Host RAM kept free for the OS and other applications.
    #[serde(default = "MemoryConfig::default_ram_reserve")]
    pub ram_reserve_bytes: u64,
    /// NVMe space kept free for logs, database and download staging.
    #[serde(default = "MemoryConfig::default_nvme_reserve")]
    pub nvme_reserve_bytes: u64,
    /// Upper bound on speculative prefetch traffic.
    #[serde(default = "MemoryConfig::default_max_prefetch")]
    pub max_prefetch_bytes: u64,
    /// VRAM safety margin kept free on every GPU, on top of the engine reserve
    /// (spec 5.4: usable = free - engine_reserve - system_reserve - safety_margin).
    #[serde(default = "MemoryConfig::default_gpu_safety_margin")]
    pub gpu_safety_margin_bytes: u64,
    /// Engine-managed tensor paging. Off unless the runtime proves it exposes
    /// safe tensor lifetimes (ADR-005).
    #[serde(default = "default_false")]
    pub allow_engine_managed_paging: bool,
}

impl MemoryConfig {
    fn default_strategy() -> String {
        "auto".to_owned()
    }
    fn default_gpu_reserve() -> u64 {
        2 * 1024 * 1024 * 1024
    }
    fn default_ram_reserve() -> u64 {
        8 * 1024 * 1024 * 1024
    }
    fn default_nvme_reserve() -> u64 {
        10 * 1024 * 1024 * 1024
    }
    fn default_max_prefetch() -> u64 {
        2 * 1024 * 1024 * 1024
    }
    fn default_gpu_safety_margin() -> u64 {
        512 * 1024 * 1024
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            strategy: Self::default_strategy(),
            gpu_reserve_bytes: Self::default_gpu_reserve(),
            ram_reserve_bytes: Self::default_ram_reserve(),
            nvme_reserve_bytes: Self::default_nvme_reserve(),
            max_prefetch_bytes: Self::default_max_prefetch(),
            gpu_safety_margin_bytes: Self::default_gpu_safety_margin(),
            allow_engine_managed_paging: false,
        }
    }
}

/// How Tensor Parallel may be used (spec §5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TensorParallelPolicy {
    /// Never plan Tensor Parallel.
    Never,
    /// Only when the runtime declares and has validated support.
    WhenSupported,
    /// Always attempt it, failing loudly when unsupported.
    Always,
}

impl TensorParallelPolicy {
    /// Stable text form used by the Settings page and the snapshot.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::WhenSupported => "when_supported",
            Self::Always => "always",
        }
    }
}

/// `[gpu]` — multi-GPU scheduling policy (spec §5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuConfig {
    /// Planner strategy name.
    #[serde(default = "GpuConfig::default_strategy")]
    pub strategy: String,
    /// Allow plans that span more than one GPU.
    #[serde(default = "default_true")]
    pub allow_multi_gpu: bool,
    /// Split one model across vendors. Off by default: cross-vendor splits are
    /// not planned without an explicit opt-in (spec §2.4).
    #[serde(default = "default_false")]
    pub allow_cross_vendor_model_split: bool,
    /// Tensor Parallel policy.
    #[serde(default = "GpuConfig::default_tp")]
    pub allow_tensor_parallel: TensorParallelPolicy,
    /// Prefer running independent models on separate GPUs.
    #[serde(default = "default_true")]
    pub prefer_task_parallel: bool,
}

impl GpuConfig {
    fn default_strategy() -> String {
        "auto".to_owned()
    }
    fn default_tp() -> TensorParallelPolicy {
        TensorParallelPolicy::WhenSupported
    }
}

impl Default for GpuConfig {
    fn default() -> Self {
        Self {
            strategy: Self::default_strategy(),
            allow_multi_gpu: true,
            allow_cross_vendor_model_split: false,
            allow_tensor_parallel: Self::default_tp(),
            prefer_task_parallel: true,
        }
    }
}

/// `[runtime]` — runtime installation and worker isolation (spec §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Install runtimes automatically when a model needs them.
    #[serde(default = "default_true")]
    pub auto_install: bool,
    /// Run every runtime in its own worker process (ADR-007).
    #[serde(default = "default_true")]
    pub isolate_workers: bool,
    /// Update runtimes without user action.
    #[serde(default = "default_false")]
    pub auto_update: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            auto_install: true,
            isolate_workers: true,
            auto_update: false,
        }
    }
}

/// Authentication mode for the local API (spec §9.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthenticationMode {
    /// Loopback only, no bearer token required.
    LocalOnly,
    /// Bearer token required; mandatory once a non-loopback bind is used.
    Token,
}

impl AuthenticationMode {
    /// Stable text form used by the Settings page and the snapshot.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalOnly => "local_only",
            Self::Token => "token",
        }
    }
}

/// `[api]` — local REST/OpenAI-compatible gateway (spec §9.2, §9.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    /// Whether to start the gateway at all.
    ///
    /// On by default because it is loopback-only: a local API that a local tool
    /// can use is the point. Turning it off is for users who want no listener.
    #[serde(default = "ApiConfig::default_enabled")]
    pub enabled: bool,
    /// Bind address. Must stay loopback unless the user opts in.
    #[serde(default = "ApiConfig::default_bind")]
    pub bind: String,
    /// Bearer token. Required by the gateway for any non-loopback bind.
    #[serde(default)]
    pub token: Option<String>,
    /// Authentication mode.
    #[serde(default = "ApiConfig::default_authentication")]
    pub authentication: AuthenticationMode,
    /// Port for the OpenAI-compatible endpoint; defaults to `[core].api_port`.
    #[serde(default)]
    pub port: Option<u16>,
}

impl ApiConfig {
    fn default_enabled() -> bool {
        true
    }
    fn default_bind() -> String {
        "127.0.0.1".to_owned()
    }
    fn default_authentication() -> AuthenticationMode {
        AuthenticationMode::LocalOnly
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: Self::default_enabled(),
            bind: Self::default_bind(),
            token: None,
            authentication: Self::default_authentication(),
            port: None,
        }
    }
}

impl Config {
    /// Parse a TOML document, rejecting unknown keys.
    pub fn from_toml_str(source: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(source)?)
    }

    /// Load a configuration file, falling back to defaults when it is absent.
    pub fn load_or_default(path: &std::path::Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let cfg = Self::from_toml_str(&text)?;
                Ok(cfg)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(ConfigError::Io {
                path: path.display().to_string(),
                source: err,
            }),
        }
    }

    /// Serialise back to TOML.
    pub fn to_toml_string(&self) -> Result<String, ConfigError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Port the REST/OpenAI-compatible gateway uses.
    pub fn effective_api_port(&self) -> u16 {
        self.api.port.unwrap_or(self.core.api_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec_example() {
        let cfg = Config::default();
        assert_eq!(cfg.core.host, "127.0.0.1");
        assert_eq!(cfg.core.api_port, 18888);
        assert_eq!(cfg.core.grpc_port, 18889);
        assert_eq!(cfg.core.log_level, "info");
        assert!(!cfg.core.start_with_login, "login items are opt-in");
        assert_eq!(cfg.paths.models, "models");
        assert_eq!(cfg.paths.cache, "cache");
        assert_eq!(cfg.paths.runtimes, "runtimes");
        assert_eq!(cfg.paths.logs, "logs");
        assert_eq!(cfg.downloads.max_concurrent_downloads, 3);
        assert!(cfg.downloads.resume);
        assert!(cfg.downloads.verify_hashes);
        assert_eq!(cfg.memory.gpu_reserve_bytes, 2_147_483_648);
        assert_eq!(cfg.memory.ram_reserve_bytes, 8_589_934_592);
        assert_eq!(cfg.memory.nvme_reserve_bytes, 10_737_418_240);
        assert_eq!(cfg.memory.max_prefetch_bytes, 2_147_483_648);
        assert!(!cfg.memory.allow_engine_managed_paging);
        assert!(!cfg.gpu.allow_cross_vendor_model_split);
        assert!(cfg.gpu.allow_multi_gpu);
        assert!(cfg.runtime.isolate_workers);
        assert!(!cfg.runtime.auto_update);
        assert_eq!(cfg.api.bind, "127.0.0.1");
    }

    #[test]
    fn empty_document_yields_all_defaults() {
        let cfg = Config::from_toml_str("").expect("empty doc parses");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_document_overrides_only_listed_keys() {
        let cfg = Config::from_toml_str(
            r#"
            [core]
            api_port = 19000
            "#,
        )
        .expect("partial doc parses");
        assert_eq!(cfg.core.api_port, 19000);
        assert_eq!(cfg.core.grpc_port, 18889, "unlisted keys keep defaults");
    }

    #[test]
    fn spec_config_example_round_trips() {
        let text = r#"
[core]
host = "127.0.0.1"
api_port = 18888
grpc_port = 18889
log_level = "info"
keep_workers_after_ui_exit = true

[paths]
models = "./models"
cache = "./cache"
runtimes = "./runtimes"
logs = "./logs"

[downloads]
max_concurrent_downloads = 3
resume = true
verify_hashes = true
temporary_directory = "./cache/downloads"

[memory]
strategy = "auto"
gpu_reserve_bytes = 2147483648
ram_reserve_bytes = 8589934592
nvme_reserve_bytes = 10737418240
max_prefetch_bytes = 2147483648
allow_engine_managed_paging = false

[gpu]
strategy = "auto"
allow_multi_gpu = true
allow_cross_vendor_model_split = false
allow_tensor_parallel = "when_supported"
prefer_task_parallel = true

[runtime]
auto_install = true
isolate_workers = true
auto_update = false

[api]
bind = "127.0.0.1"
authentication = "local_only"
"#;
        let cfg = Config::from_toml_str(text).expect("spec example parses");
        let reparsed = Config::from_toml_str(&cfg.to_toml_string().unwrap()).unwrap();
        assert_eq!(cfg, reparsed);
        assert_eq!(
            cfg.gpu.allow_tensor_parallel,
            TensorParallelPolicy::WhenSupported
        );
        assert_eq!(cfg.api.authentication, AuthenticationMode::LocalOnly);
    }

    #[test]
    fn unknown_key_is_rejected() {
        let err = Config::from_toml_str(
            "[core]
api_port = 1
surprise = true
",
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }), "got {err:?}");
    }

    #[test]
    fn api_port_falls_back_to_core_port() {
        let mut cfg = Config::default();
        assert_eq!(cfg.effective_api_port(), 18888);
        cfg.api.port = Some(20000);
        assert_eq!(cfg.effective_api_port(), 20000);
    }
}
