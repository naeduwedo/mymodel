//! `gguf config` — show the effective configuration and validation findings.

use anyhow::Result;
use gguf_engine_config::{Config, Severity};
use serde::Serialize;

use crate::commands::Context;
use crate::output;

#[derive(Debug, Serialize)]
struct ConfigReport {
    config_file: String,
    config_file_exists: bool,
    data_root: String,
    data_root_origin: String,
    paths: ResolvedPaths,
    issues: Vec<gguf_engine_config::Issue>,
    config: Config,
}

#[derive(Debug, Serialize)]
struct ResolvedPaths {
    models: String,
    cache: String,
    runtimes: String,
    logs: String,
    database: String,
    downloads: String,
}

fn origin_name(origin: gguf_engine_config::Origin) -> &'static str {
    match origin {
        gguf_engine_config::Origin::Explicit => "explicit",
        gguf_engine_config::Origin::Environment => "environment",
        gguf_engine_config::Origin::Platform => "platform",
    }
}

/// Print the configuration.
///
/// Returns `true` when the command succeeded; `false` when an error-level
/// validation finding was reported (the caller maps that to a failure exit
/// code). Showing warnings is still a success.
pub fn config(ctx: &Context, defaults: bool, json: bool) -> Result<bool> {
    let cfg = if defaults {
        Config::default()
    } else {
        ctx.config.clone()
    };
    let issues = gguf_engine_config::check(&cfg);
    let has_errors = issues.iter().any(|i| i.severity == Severity::Error);

    let report = ConfigReport {
        config_file: ctx.config_file.display().to_string(),
        config_file_exists: ctx.config_file.exists(),
        data_root: ctx.paths.root().display().to_string(),
        data_root_origin: origin_name(ctx.paths.origin()).to_owned(),
        paths: ResolvedPaths {
            models: ctx.paths.models().display().to_string(),
            cache: ctx.paths.cache().display().to_string(),
            runtimes: ctx.paths.runtimes().display().to_string(),
            logs: ctx.paths.logs().display().to_string(),
            database: ctx.paths.database().display().to_string(),
            downloads: ctx.paths.downloads().display().to_string(),
        },
        issues: issues.clone(),
        config: cfg.clone(),
    };

    output::emit(json, &report, |report| {
        println!(
            "config file : {} ({})",
            report.config_file,
            if report.config_file_exists {
                "found"
            } else {
                "not found, using defaults"
            }
        );
        println!(
            "data root   : {} ({})",
            report.data_root, report.data_root_origin
        );
        println!("models      : {}", report.paths.models);
        println!("cache       : {}", report.paths.cache);
        println!("runtimes    : {}", report.paths.runtimes);
        println!("logs        : {}", report.paths.logs);
        println!("database    : {}", report.paths.database);
        println!("downloads   : {}", report.paths.downloads);
        println!();
        println!("[core]");
        println!(
            "  host / api_port / grpc_port : {}:{} / {}",
            report.config.core.host, report.config.core.api_port, report.config.core.grpc_port
        );
        println!(
            "  log_level                    : {}",
            report.config.core.log_level
        );
        println!("[memory]");
        println!(
            "  gpu_reserve_bytes            : {} ({})",
            report.config.memory.gpu_reserve_bytes,
            output::human_bytes(report.config.memory.gpu_reserve_bytes)
        );
        println!(
            "  ram_reserve_bytes            : {} ({})",
            report.config.memory.ram_reserve_bytes,
            output::human_bytes(report.config.memory.ram_reserve_bytes)
        );
        println!(
            "  nvme_reserve_bytes           : {} ({})",
            report.config.memory.nvme_reserve_bytes,
            output::human_bytes(report.config.memory.nvme_reserve_bytes)
        );
        println!(
            "  max_prefetch_bytes           : {} ({})",
            report.config.memory.max_prefetch_bytes,
            output::human_bytes(report.config.memory.max_prefetch_bytes)
        );
        println!("[api]");
        println!(
            "  bind / authentication        : {} / {:?}",
            report.config.api.bind, report.config.api.authentication
        );
        println!();
        if report.issues.is_empty() {
            println!("validation  : ok");
        } else {
            println!("validation  : {} finding(s)", report.issues.len());
            for issue in &report.issues {
                let severity = match issue.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                };
                println!("  [{severity}] {}: {}", issue.field, issue.message);
            }
        }
    });

    Ok(!has_errors) // success unless an error-level finding exists
}

#[cfg(test)]
mod tests {
    use super::*;
    use gguf_engine_config::Origin;
    use std::path::Path;

    #[test]
    fn report_resolves_paths_under_the_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = Context::resolve_with_origin(Some(tmp.path()), Origin::Explicit, None).unwrap();
        assert!(ctx.paths.models().starts_with(tmp.path()));
        assert!(ctx.config_file.ends_with("config.toml"));
        assert!(!ctx.config_file.exists());
    }

    #[test]
    fn defaults_report_has_no_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = Context::resolve_with_origin(Some(tmp.path()), Origin::Explicit, None).unwrap();
        let issues = gguf_engine_config::check(&Config::default());
        assert!(issues.iter().all(|i| i.severity != Severity::Error));
        assert_eq!(ctx.config, Config::default());
        let _ = Path::new("x");
    }

    #[test]
    fn invalid_config_file_is_rejected_by_resolve() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.toml");
        std::fs::write(&path, "[core]\napi_port = 0\n").unwrap();
        let err = Context::resolve_with_origin(Some(tmp.path()), Origin::Explicit, Some(&path))
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("invalid configuration"),
            "{err:#}"
        );
    }
}
