//! `gguf runtimes` - report installed runtimes and their capabilities.

use anyhow::{Context as _, Result};
use gguf_engine_runtime_llama_cpp::LlamaCppAdapter;
use gguf_engine_runtime_manager::RuntimeAdapter;

/// Report every installed runtime and what it can do.
pub async fn runtimes(json: bool) -> Result<bool> {
    let discovered = LlamaCppAdapter::discover();

    if json {
        let payload = match discovered {
            Ok(adapter) => {
                let caps = adapter.capabilities().await.context("capabilities")?;
                serde_json::json!({
                    "ok": true,
                    "runtimes": [{
                        "id": "llama.cpp",
                        "directory": adapter.directory(),
                        "backends": caps.backends.iter().map(|b| b.as_str()).collect::<Vec<_>>(),
                        "multi_gpu": caps.supports_multi_gpu,
                        "gpu_offload": caps.supports_gpu_offload,
                        "streaming": caps.supports_streaming,
                        "architectures": caps.architectures,
                    }],
                })
            }
            Err(err) => {
                serde_json::json!({ "ok": true, "runtimes": [], "reason": err.to_string() })
            }
        };
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(true);
    }

    match discovered {
        Ok(adapter) => {
            let caps = adapter.capabilities().await.context("capabilities")?;
            println!("llama.cpp");
            println!("  directory   : {}", adapter.directory());
            println!(
                "  backends    : {}",
                caps.backends
                    .iter()
                    .map(|b| b.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!("  multi-gpu   : {}", caps.supports_multi_gpu);
            println!("  gpu offload : {}", caps.supports_gpu_offload);
            println!("  streaming   : {}", caps.supports_streaming);
            println!("  architectures: {}", caps.architectures.len());
            for cap in &caps.capabilities {
                let detail = cap.detail.as_deref().unwrap_or("");
                println!(
                    "    [{}] {}{}",
                    cap.status.as_str(),
                    cap.name,
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(" - {detail}")
                    }
                );
            }
        }
        Err(err) => {
            println!("no runtime installed: {err}");
            println!("set GGUF_LLAMA_CPP_DIR to the llama.cpp directory");
            return Ok(false);
        }
    }

    Ok(true)
}
