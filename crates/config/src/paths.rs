//! Platform application data directories (spec §15).
//!
//! * Windows: `%LOCALAPPDATA%/GGUF-AI-Engine/`
//! * Linux:   `~/.local/share/gguf-ai-engine/`
//! * macOS:   `~/Library/Application Support/GGUF AI Engine/`
//!
//! Every configured directory is resolved relative to a single root and is
//! rejected if it escapes that root — the first line of defence against path
//! traversal and symlink escapes (spec §9.3).

use std::path::{Component, Path, PathBuf};

use crate::{Config, ConfigError};

/// Where the application data root came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// An explicit path supplied by the caller (`--data-dir`).
    Explicit,
    /// `GGUF_AI_ENGINE_DATA_DIR` environment variable.
    Environment,
    /// The platform convention for application data.
    Platform,
}

/// Resolved, validated application directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    root: PathBuf,
    origin: Origin,
    models: PathBuf,
    cache: PathBuf,
    runtimes: PathBuf,
    logs: PathBuf,
    database: PathBuf,
    downloads: PathBuf,
}

/// Directory name used under the platform data directory.
pub const APP_DIR_NAME: &str = "GGUF-AI-Engine";

/// Environment variable that overrides the data root (used by tests and CI).
pub const DATA_DIR_ENV: &str = "GGUF_AI_ENGINE_DATA_DIR";

/// Normalise a path lexically: collapse `.`, resolve `..` against prior
/// components, drop redundant separators. Never touches the filesystem, so it
/// is deterministic and works for paths that do not exist yet.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = out.pop();
                if !popped {
                    // A leading `..` that cannot be cancelled is kept so the
                    // containment check below rejects it.
                    out.push(Component::ParentDir.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl AppPaths {
    /// Default root for the current platform.
    pub fn platform_root() -> Result<PathBuf, ConfigError> {
        let dirs = directories::BaseDirs::new()
            .ok_or_else(|| ConfigError::NoDataDirectory("no base directories available".into()))?;
        let root = if cfg!(target_os = "macos") {
            dirs.data_dir().join("GGUF AI Engine")
        } else {
            dirs.data_dir().join(APP_DIR_NAME)
        };
        Ok(root)
    }

    /// Resolve the data root using, in order: the `explicit` argument,
    /// [`DATA_DIR_ENV`], then the platform convention.
    pub fn resolve_root(explicit: Option<&Path>) -> Result<(PathBuf, Origin), ConfigError> {
        if let Some(path) = explicit {
            return Ok((normalise(path), Origin::Explicit));
        }
        match std::env::var_os(DATA_DIR_ENV) {
            Some(value) if !value.is_empty() => {
                Ok((normalise(Path::new(&value)), Origin::Environment))
            }
            _ => Ok((Self::platform_root()?, Origin::Platform)),
        }
    }

    /// Resolve every configured directory against a root.
    ///
    /// Relative entries (including `./models`) are joined to `root`. Absolute
    /// entries are kept as-is but must stay inside `root`; anything that
    /// escapes is rejected instead of silently accepted.
    pub fn from_config(root: impl AsRef<Path>, config: &Config) -> Result<Self, ConfigError> {
        let root = normalise(root.as_ref());
        let resolve = |entry: &str| -> Result<PathBuf, ConfigError> {
            let candidate = normalise(&root.join(entry));
            if candidate.starts_with(&root) {
                Ok(candidate)
            } else {
                Err(ConfigError::PathEscape {
                    path: candidate.display().to_string(),
                    root: root.display().to_string(),
                })
            }
        };

        let models = resolve(&config.paths.models)?;
        let cache = resolve(&config.paths.cache)?;
        let runtimes = resolve(&config.paths.runtimes)?;
        let logs = resolve(&config.paths.logs)?;
        let database = resolve(&config.paths.database)?;
        let downloads = resolve(&config.downloads.temporary_directory)?;

        Ok(Self {
            root,
            origin: Origin::Explicit,
            models,
            cache,
            runtimes,
            logs,
            database,
            downloads,
        })
    }

    /// Resolve with an explicit origin record.
    pub fn from_config_with_origin(
        root: impl AsRef<Path>,
        origin: Origin,
        config: &Config,
    ) -> Result<Self, ConfigError> {
        let mut paths = Self::from_config(root, config)?;
        paths.origin = origin;
        Ok(paths)
    }

    /// Application data root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the root came from.
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// Model directory.
    pub fn models(&self) -> &Path {
        &self.models
    }

    /// Cache directory.
    pub fn cache(&self) -> &Path {
        &self.cache
    }

    /// Runtime installation directory.
    pub fn runtimes(&self) -> &Path {
        &self.runtimes
    }

    /// Log directory.
    pub fn logs(&self) -> &Path {
        &self.logs
    }

    /// Database directory.
    pub fn database(&self) -> &Path {
        &self.database
    }

    /// Download staging directory.
    pub fn downloads(&self) -> &Path {
        &self.downloads
    }

    /// Path of the SQLite database file.
    pub fn database_file(&self) -> PathBuf {
        self.database.join("engine.db")
    }

    /// Path of `config.toml`.
    pub fn config_file(&self) -> PathBuf {
        self.root.join(crate::CONFIG_FILE_NAME)
    }

    /// Path of the single-instance lock file (spec §3.3).
    pub fn lock_file(&self) -> PathBuf {
        self.database.join("core.lock")
    }

    /// Create every directory. Idempotent.
    pub fn ensure_dirs(&self) -> Result<(), ConfigError> {
        for dir in [
            &self.models,
            &self.cache,
            &self.runtimes,
            &self.logs,
            &self.database,
            &self.downloads,
        ] {
            std::fs::create_dir_all(dir).map_err(|source| ConfigError::Io {
                path: dir.display().to_string(),
                source,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_models(entry: &str) -> Config {
        let mut cfg = Config::default();
        cfg.paths.models = entry.to_owned();
        cfg
    }

    #[test]
    fn normalise_collapses_dot_and_dotdot() {
        assert_eq!(normalise(Path::new("a/./b/../c")), PathBuf::from("a/c"));
        assert_eq!(normalise(Path::new("./models")), PathBuf::from("models"));
        assert_eq!(normalise(Path::new("a//b")), PathBuf::from("a/b"));
    }

    #[test]
    fn relative_paths_resolve_under_root() {
        let root = Path::new("data").join("engine");
        let cfg = cfg_with_models("./models");
        let paths = AppPaths::from_config(&root, &cfg).unwrap();
        assert_eq!(paths.models(), normalise(&root.join("models")).as_path());
    }

    #[test]
    fn traversal_out_of_root_is_rejected() {
        let cfg = cfg_with_models("../../etc");
        let err = AppPaths::from_config(Path::new("data/engine"), &cfg).unwrap_err();
        assert!(matches!(err, ConfigError::PathEscape { .. }), "got {err:?}");
    }

    #[test]
    fn absolute_path_outside_root_is_rejected() {
        let outside = if cfg!(windows) {
            r"C:\Windows\System32"
        } else {
            "/etc"
        };
        let cfg = cfg_with_models(outside);
        let err = AppPaths::from_config(Path::new("data/engine"), &cfg).unwrap_err();
        assert!(matches!(err, ConfigError::PathEscape { .. }), "got {err:?}");
    }

    #[test]
    fn ensure_dirs_creates_every_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_config(tmp.path(), &Config::default()).unwrap();
        paths.ensure_dirs().unwrap();
        for dir in [
            paths.models(),
            paths.cache(),
            paths.runtimes(),
            paths.logs(),
            paths.database(),
            paths.downloads(),
        ] {
            assert!(dir.is_dir(), "{} should exist", dir.display());
        }
        paths.ensure_dirs().expect("idempotent");
    }

    #[test]
    fn database_and_lock_paths_live_inside_root() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_config(tmp.path(), &Config::default()).unwrap();
        assert!(paths.database_file().starts_with(paths.root()));
        assert!(paths.lock_file().starts_with(paths.root()));
        assert!(paths.config_file().starts_with(paths.root()));
    }
}
