//! CLI command implementations.

mod ask;
mod benchmark;
mod chat;
mod config;
mod doctor;
mod hardware;
mod inspect;
mod memory;
mod modalities;
mod models;
mod overview;
mod plan;
mod runtimes;
mod serve;
mod version;
mod vision;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gguf_engine_config::{AppPaths, Config};

pub use ask::ask;
pub use benchmark::benchmark;
pub use chat::chat;
pub use config::config;
pub use doctor::doctor;
pub use hardware::hardware;
pub use inspect::inspect;
pub use memory::memory;
pub use modalities::modalities_cmd;
pub use models::{list, pull, search};
pub use overview::overview;
pub use plan::plan_cmd;
pub use runtimes::runtimes;
pub use serve::serve;
pub use version::version;
pub use vision::vision;

/// Resolved environment shared by every command.
#[derive(Debug)]
pub struct Context {
    /// Typed configuration after parsing and validation.
    pub config: Config,
    /// Resolved directories.
    pub paths: AppPaths,
    /// Configuration file actually used (or that would be used).
    pub config_file: PathBuf,
    /// Non-fatal validation findings.
    pub warnings: Vec<gguf_engine_config::Issue>,
}

impl Context {
    /// Data directory in effect for this invocation, and where it came from.
    ///
    /// The origin matters: "explicit" (--data-dir), "environment"
    /// (GGUF_AI_ENGINE_DATA_DIR) or "platform" (the OS default) is shown by
    /// `gguf config` and by the Settings page so a user can tell why a
    /// directory was chosen.
    pub fn data_dir(cli: &crate::Cli) -> Result<(PathBuf, gguf_engine_config::Origin)> {
        AppPaths::resolve_root(cli.data_dir.as_deref()).context("resolving data directory")
    }

    /// Resolve configuration and directories with an explicit root origin.
    pub fn resolve_with_origin(
        data_dir: Option<&Path>,
        origin: gguf_engine_config::Origin,
        config_file: Option<&Path>,
    ) -> Result<Self> {
        let (root, _) = AppPaths::resolve_root(data_dir).context("resolving data directory")?;
        let config_file = match config_file {
            Some(path) => path.to_path_buf(),
            None => root.join(gguf_engine_config::CONFIG_FILE_NAME),
        };

        let config = Config::load_or_default(&config_file)
            .with_context(|| format!("loading configuration from {}", config_file.display()))?;
        let warnings = gguf_engine_config::check(&config);
        gguf_engine_config::validate(&config).context("configuration is invalid")?;

        let paths = AppPaths::from_config_with_origin(&root, origin, &config)
            .context("resolving configured directories")?;

        Ok(Self {
            config,
            paths,
            config_file,
            warnings,
        })
    }
}
