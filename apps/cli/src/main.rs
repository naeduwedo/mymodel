//! `gguf` — command line interface for GGUF AI Engine.
//!
//! The CLI is the first client of the core and the primary tool for support:
//! `gguf doctor` must be able to explain a broken installation without a UI.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use gguf_engine_telemetry::LogFormat;

mod commands;
mod output;

/// Command line entry point.
#[derive(Debug, Parser)]
#[command(
    name = "gguf",
    version,
    about = "GGUF AI Engine — local GGUF model manager and inference platform",
    long_about = None
)]
struct Cli {
    /// Override the application data directory.
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Override the configuration file path.
    #[arg(long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Emit machine readable JSON instead of text.
    #[arg(long, global = true)]
    json: bool,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

/// Log format for commands that run the engine in-process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// Human readable logs.
    Pretty,
    /// One JSON object per line.
    Json,
}

#[derive(Debug, Subcommand)]
enum ModelsAction {
    /// Search the Hub for GGUF repositories.
    Search {
        /// Query text.
        query: String,
        /// Maximum number of results.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// List models recorded in the local registry.
    List,
    /// Download, verify and register one model file.
    Pull {
        /// Repository, e.g. ggml-org/models.
        repo: String,
        /// File inside the repository. Required when the repository has several.
        #[arg(long)]
        file: Option<String>,
        /// Revision: a branch, tag or commit.
        #[arg(long, default_value = "main")]
        revision: String,
    },
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the resolved configuration and its validation findings.
    Config {
        /// Print the default configuration instead of the on-disk one.
        #[arg(long)]
        defaults: bool,
    },
    /// Report core health and environment diagnostics.
    Doctor,
    /// Print the Overview and Settings snapshot the native UIs display.
    Overview,
    /// Print the hardware inventory (CPU, memory, storage, GPUs).
    Hardware,
    /// Parse GGUF files and report what they contain.
    Inspect {
        /// Files to inspect.
        #[arg(required = true, value_name = "FILE")]
        files: Vec<PathBuf>,
    },
    /// Search, list and pull models from Hugging Face.
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },
    /// Report installed runtimes and their capabilities.
    Runtimes,
    /// Show how a model would be scheduled on this machine.
    Plan {
        /// Weight size in GiB.
        #[arg(long, default_value_t = 8.0)]
        weights_gib: f64,
        /// KV cache size in GiB.
        #[arg(long, default_value_t = 1.0)]
        kv_gib: f64,
        /// Context size in tokens (informational).
        #[arg(long, default_value_t = 4096)]
        context: u32,
    },
    /// Show the VRAM / RAM / NVMe tier picture.
    Memory,
    /// Run a streaming chat completion through a local runtime.
    Chat {
        /// Path to the GGUF model file.
        #[arg(long, value_name = "MODEL")]
        model: String,
        /// The user prompt.
        #[arg(short, long)]
        prompt: String,
        /// Maximum tokens to generate.
        #[arg(long, default_value_t = 128)]
        max_tokens: u32,
        /// Sampling temperature.
        #[arg(long, default_value_t = 0.7)]
        temperature: f32,
    },
    /// Run the core daemon in the foreground.
    Serve {
        /// Log format.
        #[arg(long, value_enum, default_value_t = Format::Pretty)]
        log_format: Format,
    },
    /// Print version and build information.
    Version,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("gguf: failed to start the async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };

    let level = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    // Logs go to stderr so that --json output on stdout stays parseable.
    if let Err(err) = gguf_engine_telemetry::tracing_setup(level, LogFormat::Pretty) {
        eprintln!("gguf: logger already installed: {err}");
    }

    let result = runtime.block_on(run(cli));
    match result {
        Ok(code) => code,
        Err(err) => {
            if json {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": format!("{err:#}"),
                });
                println!("{payload}");
            } else {
                eprintln!("gguf: error: {err:#}");
                eprintln!("gguf: run `gguf doctor` for diagnostics");
            }
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
    // `--data-dir` is a global flag, so it may legally follow the subcommand.
    let (data_dir, origin) = commands::Context::data_dir(&cli)?;
    let ctx = commands::Context::resolve_with_origin(
        Some(data_dir.as_path()),
        origin,
        cli.config.as_deref(),
    )?;

    match cli.command {
        Command::Config { defaults } => {
            let ok = commands::config(&ctx, defaults, cli.json)?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Doctor => {
            let ok = commands::doctor(&ctx, cli.json).await?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Overview => {
            let ok = commands::overview(&ctx, cli.json).await?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Hardware => {
            let ok = commands::hardware(&ctx, cli.json).await?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Inspect { files } => {
            let ok = commands::inspect(&files, cli.json)?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Models { action } => match action {
            ModelsAction::Search { query, limit } => {
                let ok = commands::search(&ctx, &query, limit, cli.json).await?;
                Ok(if ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                })
            }
            ModelsAction::List => {
                let ok = commands::list(&ctx, cli.json)?;
                Ok(if ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                })
            }
            ModelsAction::Pull {
                repo,
                file,
                revision,
            } => {
                let ok = commands::pull(&ctx, &repo, file.as_deref(), &revision, cli.json).await?;
                Ok(if ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                })
            }
        },
        Command::Runtimes => {
            let ok = commands::runtimes(cli.json).await?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Plan {
            weights_gib,
            kv_gib,
            context,
        } => {
            let ok = commands::plan_cmd(&ctx, weights_gib, kv_gib, context, cli.json)?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Memory => {
            let ok = commands::memory(&ctx, cli.json)?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Chat {
            model,
            prompt,
            max_tokens,
            temperature,
        } => {
            let ok = commands::chat(&model, &prompt, max_tokens, temperature).await?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Command::Serve { log_format } => {
            let _ = log_format;
            commands::serve(&ctx, cli.json).await
        }
        Command::Version => {
            commands::version(cli.json)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_options_are_accepted_before_and_after_the_subcommand() {
        let a = Cli::try_parse_from(["gguf", "--json", "version"]).unwrap();
        assert!(a.json);
        let b = Cli::try_parse_from(["gguf", "version", "--json"]).unwrap();
        assert!(b.json);
    }

    #[test]
    fn verbose_is_counted() {
        let cli = Cli::try_parse_from(["gguf", "-vvv", "doctor"]).unwrap();
        assert_eq!(cli.verbose, 3);
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["gguf", "frobnicate"]).is_err());
    }

    #[test]
    fn serve_accepts_a_log_format() {
        let cli = Cli::try_parse_from(["gguf", "serve", "--log-format", "json"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Serve {
                log_format: Format::Json
            }
        ));
    }
}
