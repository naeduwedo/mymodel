//! `gguf benchmark` - measure tok/s and VRAM per offload strategy.
//!
//! The numbers are produced here rather than quoted in a document, so anyone
//! can re-run them on their own machine and compare.

use anyhow::{Context as _, Result};
use gguf_engine_runtime_llama_cpp::LlamaCppAdapter;
use gguf_engine_runtime_manager::benchmark::{
    BenchmarkReport, PROMPT, RunSample, StrategyReport, TOKENS_PER_RUN, peak_vram_during,
    strategy_layers,
};
use gguf_engine_runtime_manager::{ChatMessage, ChatRequest, LoadPlan, RuntimeAdapter};
use tokio_stream::StreamExt;

use crate::output;

/// Run the benchmark.
pub async fn benchmark(model: &str, runs: usize, json: bool) -> Result<bool> {
    let adapter = LlamaCppAdapter::discover()
        .context("locating llama.cpp; set GGUF_LLAMA_CPP_DIR if it is not on PATH")?;

    if !json {
        println!("model    : {model}");
        println!("prompt   : {PROMPT}");
        println!("per run  : {TOKENS_PER_RUN} tokens, temperature 0");
        println!("runs     : {runs} measured + 1 warm-up per strategy");
        println!();
    }

    let mut strategies = Vec::new();

    for (gpu_layers, label) in strategy_layers() {
        if !json {
            print!("{label} ({gpu_layers} layers) ... ");
            use std::io::Write as _;
            let _ = std::io::stdout().flush();
        }

        // Baseline before this strategy loads anything, so the reported VRAM is
        // what the model added rather than what the desktop was already using.
        let baseline = gguf_engine_runtime_manager::benchmark::sample_vram_bytes();

        let plan = LoadPlan {
            model_path: model.to_owned(),
            gpu_layers: Some(gpu_layers),
            context_size: 2048,
        };

        // Load time is measured, not guessed: it is the number a user feels.
        let load_started = std::time::Instant::now();
        let instance = match adapter.load(&plan).await {
            Ok(instance) => instance,
            Err(err) => {
                if !json {
                    println!("skipped ({err})");
                }
                continue;
            }
        };
        let load_ms = load_started.elapsed().as_millis() as u64;

        // Warm-up: pays for page-cache misses and CUDA context creation, and
        // reporting it as steady state would flatter the result.
        let _ = run_once(&adapter, &instance).await;

        let mut samples = Vec::new();
        for _ in 0..runs.max(1) {
            match run_once(&adapter, &instance).await {
                Ok(sample) => samples.push(sample),
                Err(err) => {
                    if !json {
                        println!("run failed: {err}");
                    }
                }
            }
        }

        let _ = adapter.unload(instance).await;

        let report = StrategyReport {
            strategy: label.to_owned(),
            gpu_layers,
            load_ms,
            baseline_vram_bytes: baseline,
            runs: samples,
        };

        if !json {
            println!(
                "{:.1} tok/s (best {:.1}), load {} ms, VRAM +{}",
                report.median_tokens_per_second(),
                report.best_tokens_per_second(),
                report.load_ms,
                report
                    .added_vram_bytes()
                    .map(output::human_bytes)
                    .unwrap_or_else(|| "unavailable".to_owned())
            );
        }

        strategies.push(report);
    }

    let report = BenchmarkReport {
        model: model.to_owned(),
        prompt: PROMPT.to_owned(),
        tokens_per_run: TOKENS_PER_RUN,
        runs_per_strategy: runs,
        strategies,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(!report.strategies.is_empty());
    }

    if report.strategies.is_empty() {
        println!("no strategy could run; check the runtime and the model");
        return Ok(false);
    }

    if let Some(fastest) = report.fastest() {
        println!();
        println!(
            "fastest  : {} at {:.1} tok/s",
            fastest.strategy,
            fastest.median_tokens_per_second()
        );
    }

    Ok(true)
}

/// One measured generation.
async fn run_once(
    adapter: &LlamaCppAdapter,
    instance: &gguf_engine_runtime_manager::RuntimeInstance,
) -> Result<RunSample> {
    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: "user".to_owned(),
            content: PROMPT.to_owned(),
        }],
        max_tokens: TOKENS_PER_RUN,
        // Temperature 0: sampling would make the token count vary between runs.
        temperature: 0.0,
        stream: true,
    };

    let (result, peak) = peak_vram_during(async {
        let started = std::time::Instant::now();
        let mut stream = adapter.chat(instance, &request).await?;
        let mut tokens = 0u32;
        while let Some(item) = stream.next().await {
            let token = item?;
            if !token.content.is_empty() {
                tokens += 1;
            }
        }
        Ok::<_, gguf_engine_runtime_manager::RuntimeError>((started.elapsed(), tokens))
    })
    .await;

    let (elapsed, tokens) = result?;
    Ok(RunSample::new(elapsed, tokens, peak))
}
