//! Benchmark harness (spec §11, Task 7 gap: tok/s and VRAM peak per strategy).
//!
//! Why this exists as code rather than a table in a document: a number in a
//! document cannot be re-measured on the reader's machine, and "llama.cpp is
//! fast" is not a measurement.
//!
//! Rules the harness follows, so its numbers mean something:
//!
//! * **the same prompt every run** - a benchmark that changes its input measures
//!   nothing;
//! * **temperature 0** - sampling makes the token count vary between runs;
//! * **warm-up excluded** - the first run pays for page-cache misses and CUDA
//!   context creation, and reporting it as steady state would flatter the result;
//! * **VRAM sampled during generation, not after** - the peak happens while
//!   tokens are being produced, and by the time the call returns it has dropped;
//! * **the strategy is reported with the number** - "40 tok/s" is meaningless
//!   without saying whether the layers were on the GPU.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// The prompt every run uses. Fixed, short, and not from a story corpus, so a
/// model cannot have memorised the continuation.
pub const PROMPT: &str = "List three colours and nothing else.";

/// How many tokens each measured run generates.
pub const TOKENS_PER_RUN: u32 = 32;

/// One measured run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSample {
    /// Wall-clock milliseconds from request to last token.
    pub elapsed_ms: u64,
    /// Tokens the runtime reported generating.
    pub tokens: u32,
    /// Tokens per second for this run.
    pub tokens_per_second: f64,
    /// Peak VRAM observed on any device during this run, in bytes.
    pub peak_vram_bytes: Option<u64>,
}

impl RunSample {
    /// Build a sample from a measured span and a token count.
    pub fn new(elapsed: Duration, tokens: u32, peak_vram_bytes: Option<u64>) -> Self {
        let seconds = elapsed.as_secs_f64();
        Self {
            elapsed_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
            tokens,
            tokens_per_second: if seconds > 0.0 {
                f64::from(tokens) / seconds
            } else {
                0.0
            },
            peak_vram_bytes,
        }
    }
}

/// The result for one strategy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyReport {
    /// Human-readable strategy, e.g. "all layers on GPU".
    pub strategy: String,
    /// `--n-gpu-layers` value used.
    pub gpu_layers: u32,
    /// Time to load the model and become healthy.
    pub load_ms: u64,
    /// Device VRAM in use before this strategy started, in bytes.
    ///
    /// Recorded because the peak alone is misleading: on a machine with a
    /// browser open, "peak 2.6 GiB" for a CPU-only run is somebody else's
    /// memory. The useful number is the difference.
    pub baseline_vram_bytes: Option<u64>,
    /// Measured runs, warm-up excluded.
    pub runs: Vec<RunSample>,
}

impl StrategyReport {
    /// Median tokens per second across the measured runs.
    ///
    /// Median rather than mean: one run that stalls on a driver hiccup would
    /// drag a mean down and misreport the steady state.
    pub fn median_tokens_per_second(&self) -> f64 {
        median(self.runs.iter().map(|run| run.tokens_per_second))
    }

    /// Best tokens per second across the measured runs.
    pub fn best_tokens_per_second(&self) -> f64 {
        self.runs
            .iter()
            .map(|run| run.tokens_per_second)
            .fold(0.0_f64, f64::max)
    }

    /// Highest VRAM peak observed across runs.
    pub fn peak_vram_bytes(&self) -> Option<u64> {
        self.runs.iter().filter_map(|run| run.peak_vram_bytes).max()
    }

    /// VRAM this strategy actually added, above the pre-load baseline.
    ///
    /// `None` when either figure is unknown: a difference against an absent
    /// measurement is not a measurement.
    pub fn added_vram_bytes(&self) -> Option<u64> {
        let peak = self.peak_vram_bytes()?;
        let baseline = self.baseline_vram_bytes?;
        Some(peak.saturating_sub(baseline))
    }
}

/// A full benchmark session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// Model the numbers describe.
    pub model: String,
    /// Prompt used, recorded so the number is reproducible.
    pub prompt: String,
    /// Tokens per run.
    pub tokens_per_run: u32,
    /// Measured runs per strategy.
    pub runs_per_strategy: usize,
    /// One entry per strategy that was measured.
    pub strategies: Vec<StrategyReport>,
}

impl BenchmarkReport {
    /// The fastest strategy, when anything was measured.
    pub fn fastest(&self) -> Option<&StrategyReport> {
        self.strategies.iter().max_by(|a, b| {
            a.median_tokens_per_second()
                .partial_cmp(&b.median_tokens_per_second())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

/// The strategies a benchmark tries, as `--n-gpu-layers` values.
///
/// 0 is CPU-only, a large number is "all layers", and the middle value is the
/// interesting one: it shows what a partial offload costs.
pub fn strategy_layers() -> Vec<(u32, &'static str)> {
    vec![
        (0, "CPU only"),
        (16, "partial GPU offload"),
        (999, "all layers on GPU"),
    ]
}

fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut sorted: Vec<f64> = values.collect();
    if sorted.is_empty() {
        return 0.0;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

/// Sample VRAM usage from `nvidia-smi`, returning the peak across devices.
///
/// Returns `None` when no NVIDIA tool is present: an absent measurement is
/// reported as absent rather than as zero.
pub fn sample_vram_bytes() -> Option<u64> {
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let peak_mib = text
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .max()?;
    Some(peak_mib * 1024 * 1024)
}

/// Poll VRAM while `body` runs, returning the highest value seen.
///
/// Polls every 25 ms: fast enough to catch a peak inside a multi-second
/// generation, cheap enough not to perturb it.
pub async fn peak_vram_during<F, T>(body: F) -> (T, Option<u64>)
where
    F: std::future::Future<Output = T>,
{
    let mut peak: Option<u64> = sample_vram_bytes();
    tokio::pin!(body);

    let mut ticker = tokio::time::interval(Duration::from_millis(25));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            result = &mut body => return (result, peak),
            _ = ticker.tick() => {
                if let Some(used) = sample_vram_bytes() {
                    peak = Some(peak.map_or(used, |current| current.max(used)));
                }
            }
        }
    }
}

/// Time a future, for load measurements.
pub async fn timed<F, T>(body: F) -> (T, Duration)
where
    F: std::future::Future<Output = T>,
{
    let started = Instant::now();
    let value = body.await;
    (value, started.elapsed())
}
