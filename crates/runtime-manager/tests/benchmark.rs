//! Benchmark tests.
//!
//! The statistics are pure and tested directly; the VRAM sampler is tested for
//! its "absent is absent" behaviour rather than for a number, because a CI
//! machine has no GPU and a test that demands one would be skipped everywhere.

use std::time::Duration;

use gguf_engine_runtime_manager::benchmark::{
    BenchmarkReport, RunSample, StrategyReport, peak_vram_during, strategy_layers,
};

fn sample(tokens: u32, millis: u64) -> RunSample {
    RunSample::new(Duration::from_millis(millis), tokens, Some(1024))
}

#[test]
fn tokens_per_second_is_computed_from_the_measured_span() {
    // 32 tokens in 2 seconds is 16 tok/s, not 32/2 rounded or anything else.
    let sample = sample(32, 2_000);
    assert_eq!(sample.tokens, 32);
    assert_eq!(sample.elapsed_ms, 2_000);
    assert!((sample.tokens_per_second - 16.0).abs() < 1e-9);
}

#[test]
fn a_zero_length_run_does_not_divide_by_zero() {
    let sample = RunSample::new(Duration::ZERO, 5, None);
    assert_eq!(sample.tokens_per_second, 0.0);
}

#[test]
fn the_median_ignores_a_single_stalled_run() {
    // One run stalled on a driver hiccup; the median is the honest steady state.
    let report = StrategyReport {
        strategy: "all layers on GPU".to_owned(),
        gpu_layers: 999,
        load_ms: 1200,
        baseline_vram_bytes: None,
        runs: vec![sample(32, 800), sample(32, 810), sample(32, 40_000)],
    };

    let median = report.median_tokens_per_second();
    assert!(
        (median - 40.0).abs() < 0.5,
        "median should be the two healthy runs, got {median}"
    );
    assert!(report.best_tokens_per_second() > median);
}

#[test]
fn an_even_number_of_runs_averages_the_middle_two() {
    let report = StrategyReport {
        strategy: "CPU only".to_owned(),
        gpu_layers: 0,
        load_ms: 100,
        baseline_vram_bytes: None,
        runs: vec![sample(10, 1_000), sample(20, 1_000)],
    };
    // 10 tok/s and 20 tok/s -> 15.
    assert!((report.median_tokens_per_second() - 15.0).abs() < 1e-9);
}

#[test]
fn an_empty_report_reports_zero_rather_than_panicking() {
    let report = StrategyReport {
        strategy: "CPU only".to_owned(),
        gpu_layers: 0,
        load_ms: 0,
        baseline_vram_bytes: None,
        runs: Vec::new(),
    };
    assert_eq!(report.median_tokens_per_second(), 0.0);
    assert_eq!(report.best_tokens_per_second(), 0.0);
    assert_eq!(report.peak_vram_bytes(), None);
}

#[test]
fn the_peak_vram_is_the_highest_across_runs_not_the_last() {
    let report = StrategyReport {
        strategy: "all layers on GPU".to_owned(),
        gpu_layers: 999,
        load_ms: 0,
        baseline_vram_bytes: None,
        runs: vec![
            RunSample::new(Duration::from_millis(100), 1, Some(8 * 1024 * 1024 * 1024)),
            RunSample::new(Duration::from_millis(100), 1, Some(3 * 1024 * 1024 * 1024)),
        ],
    };
    assert_eq!(report.peak_vram_bytes(), Some(8 * 1024 * 1024 * 1024));
}

#[test]
fn the_fastest_strategy_is_the_one_with_the_highest_median() {
    let report = BenchmarkReport {
        model: "test/model".to_owned(),
        prompt: "hello".to_owned(),
        tokens_per_run: 32,
        runs_per_strategy: 2,
        strategies: vec![
            StrategyReport {
                strategy: "CPU only".to_owned(),
                gpu_layers: 0,
                load_ms: 100,
                baseline_vram_bytes: None,
                runs: vec![sample(32, 4_000), sample(32, 4_100)],
            },
            StrategyReport {
                strategy: "all layers on GPU".to_owned(),
                gpu_layers: 999,
                load_ms: 2_000,
                baseline_vram_bytes: None,
                runs: vec![sample(32, 800), sample(32, 820)],
            },
        ],
    };

    let fastest = report.fastest().expect("one strategy is faster");
    assert_eq!(fastest.strategy, "all layers on GPU");
}

#[test]
fn an_empty_benchmark_has_no_fastest_strategy() {
    let report = BenchmarkReport {
        model: "test/model".to_owned(),
        prompt: String::new(),
        tokens_per_run: 0,
        runs_per_strategy: 0,
        strategies: Vec::new(),
    };
    assert!(report.fastest().is_none());
}

#[test]
fn the_strategy_ladder_covers_cpu_partial_and_full_offload() {
    let layers = strategy_layers();
    assert_eq!(layers.len(), 3);
    assert_eq!(layers[0].0, 0, "the first strategy must be CPU only");
    assert!(layers.iter().any(|(value, _)| *value > 0 && *value < 100));
    assert!(layers.iter().any(|(value, _)| *value >= 999));
}

#[test]
fn added_vram_is_the_difference_from_the_baseline() {
    let report = StrategyReport {
        strategy: "all layers on GPU".to_owned(),
        gpu_layers: 999,
        load_ms: 0,
        baseline_vram_bytes: Some(2 * 1024 * 1024 * 1024),
        runs: vec![RunSample::new(
            Duration::from_millis(100),
            1,
            Some(3 * 1024 * 1024 * 1024),
        )],
    };
    assert_eq!(report.added_vram_bytes(), Some(1024 * 1024 * 1024));
}

#[test]
fn added_vram_is_unknown_when_the_baseline_is_unknown() {
    // A difference against an absent measurement is not a measurement.
    let report = StrategyReport {
        strategy: "CPU only".to_owned(),
        gpu_layers: 0,
        load_ms: 0,
        baseline_vram_bytes: None,
        runs: vec![RunSample::new(
            Duration::from_millis(100),
            1,
            Some(1024 * 1024 * 1024),
        )],
    };
    assert_eq!(report.added_vram_bytes(), None);
}

#[tokio::test]
async fn the_vram_sampler_reports_absent_rather_than_zero_without_a_gpu() {
    // On a machine with no nvidia-smi this is None; with one it is a positive
    // number. Both are acceptable, and neither may be zero-by-default.
    if let Some(bytes) = gguf_engine_runtime_manager::benchmark::sample_vram_bytes() {
        assert!(bytes > 0, "a sampled value must be real");
    }
}

#[tokio::test]
async fn polling_vram_returns_the_body_result_and_a_peak() {
    let (value, peak) = peak_vram_during(async {
        tokio::time::sleep(Duration::from_millis(60)).await;
        42
    })
    .await;

    assert_eq!(
        value, 42,
        "the body result must survive the polling wrapper"
    );
    // Peak may be None on a machine with no GPU tool; it must never be Some(0).
    if let Some(bytes) = peak {
        assert!(bytes > 0);
    }
}
