//! Planner and ledger tests, using this project's real two-GPU machine as the
//! fixture: RTX 4090 (24 GiB) + RTX 3060 (12 GiB), both NVIDIA/CUDA.

use gguf_engine_config::GpuConfig;
use gguf_engine_gpu_manager::{
    ExecutionStrategy, GpuBudget, GpuLedger, ModelMemoryRequirement, ReserveRequest, RuntimeFacts,
    plan, total_usable,
};
use gguf_engine_hardware::{ComputeBackend, GpuDevice, GpuVendor};

const GIB: u64 = 1024 * 1024 * 1024;
const ENGINE_RESERVE: u64 = 2 * GIB;
const SAFETY_MARGIN: u64 = GIB / 2;

fn device(id: &str, name: &str, total_gib: u64, free_gib: u64, vendor: GpuVendor) -> GpuDevice {
    GpuDevice {
        id: id.to_owned(),
        name: name.to_owned(),
        vendor,
        backend: if vendor == GpuVendor::Nvidia {
            ComputeBackend::Cuda
        } else {
            ComputeBackend::Rocm
        },
        total_vram_bytes: Some(total_gib * GIB),
        free_vram_bytes: Some(free_gib * GIB),
        memory_bandwidth_bytes_sec: None,
        compute_capability: None,
        pci_bus_id: Some(id.to_owned()),
        pcie_generation: Some(4),
        pcie_lanes: Some(16),
        numa_node: None,
        temperature_c: None,
        utilization_percent: None,
        power_watts: None,
        peer_access: Vec::new(),
        driver_version: Some("610.88".to_owned()),
    }
}

fn two_nvidia() -> (Vec<GpuDevice>, Vec<GpuBudget>) {
    let devices = vec![
        device("0000:01:00.0", "RTX 4090", 24, 22, GpuVendor::Nvidia),
        device("0000:07:00.0", "RTX 3060", 12, 11, GpuVendor::Nvidia),
    ];
    let budgets = devices
        .iter()
        .map(|d| GpuBudget::for_device(d, ENGINE_RESERVE, SAFETY_MARGIN))
        .collect();
    (devices, budgets)
}

fn requirement(weights_gib: u64, kv_gib: u64) -> ModelMemoryRequirement {
    ModelMemoryRequirement {
        weights: weights_gib * GIB,
        kv_cache: kv_gib * GIB,
        activations: GIB / 2,
        workspace: GIB / 4,
    }
}

fn full_runtime() -> RuntimeFacts {
    RuntimeFacts {
        supports_gpu_offload: true,
        supports_tensor_parallel: false,
        supports_cpu_offload: true,
    }
}

// ------------------------------------------------------------- budgets -----

#[test]
fn budget_subtracts_engine_reserve_and_safety_margin() {
    let device = device("0", "RTX 4090", 24, 22, GpuVendor::Nvidia);
    let budget = GpuBudget::for_device(&device, ENGINE_RESERVE, SAFETY_MARGIN);

    assert_eq!(budget.observed_free, 22 * GIB);
    assert_eq!(budget.usable, 22 * GIB - ENGINE_RESERVE - SAFETY_MARGIN);
}

#[test]
fn unknown_free_vram_yields_zero_usable_not_a_guess() {
    let mut device = device("0", "Card", 24, 22, GpuVendor::Nvidia);
    device.free_vram_bytes = None;
    let budget = GpuBudget::for_device(&device, ENGINE_RESERVE, SAFETY_MARGIN);
    assert_eq!(
        budget.usable, 0,
        "scheduling against unknown memory is not allowed"
    );
}

#[test]
fn budgets_are_never_summed_into_a_single_pool_by_the_budget_type() {
    // Each budget stands alone; only the planner may add them, and it must say so.
    let (_, budgets) = two_nvidia();
    assert_eq!(budgets.len(), 2);
    assert_eq!(
        total_usable(&budgets),
        budgets[0].usable + budgets[1].usable
    );
}

// ------------------------------------------------------------- planner -----

#[test]
fn a_model_that_fits_on_one_gpu_is_placed_on_the_largest() {
    let (devices, budgets) = two_nvidia();
    let req = requirement(8, 1);
    let plan = plan(
        &devices,
        &budgets,
        &req,
        &GpuConfig::default(),
        full_runtime(),
    )
    .unwrap();

    assert_eq!(plan.strategy, ExecutionStrategy::SingleGpu);
    assert_eq!(
        plan.devices,
        vec!["0000:01:00.0"],
        "the 4090 is the larger card"
    );
    assert_eq!(plan.reserved_bytes, vec![req.total()]);
}

#[test]
fn a_model_too_large_for_one_gpu_is_split_across_two() {
    let (devices, budgets) = two_nvidia();
    // 20 GiB weights will not fit in ~19.5 GiB usable on the 4090.
    let req = requirement(20, 2);
    let plan = plan(
        &devices,
        &budgets,
        &req,
        &GpuConfig::default(),
        full_runtime(),
    )
    .unwrap();

    assert_eq!(plan.strategy, ExecutionStrategy::LayerSplit);
    assert_eq!(plan.devices.len(), 2);
    assert_eq!(plan.reserved_bytes.iter().sum::<u64>(), req.total());
}

#[test]
fn tensor_parallel_is_used_only_when_the_runtime_declares_it() {
    let (devices, budgets) = two_nvidia();
    let req = requirement(20, 2);

    let mut runtime = full_runtime();
    runtime.supports_tensor_parallel = true;
    let with_tp = plan(&devices, &budgets, &req, &GpuConfig::default(), runtime).unwrap();
    assert_eq!(with_tp.strategy, ExecutionStrategy::TensorParallel);

    let without_tp = plan(
        &devices,
        &budgets,
        &req,
        &GpuConfig::default(),
        full_runtime(),
    )
    .unwrap();
    assert_eq!(
        without_tp.strategy,
        ExecutionStrategy::LayerSplit,
        "without runtime support the planner must not propose tensor parallel"
    );
}

#[test]
fn cross_vendor_splitting_is_refused_unless_enabled() {
    let devices = vec![
        device("nvidia", "RTX 4090", 24, 22, GpuVendor::Nvidia),
        device("amd", "Radeon", 32, 30, GpuVendor::Amd),
    ];
    let budgets: Vec<GpuBudget> = devices
        .iter()
        .map(|d| GpuBudget::for_device(d, ENGINE_RESERVE, SAFETY_MARGIN))
        .collect();

    let req = requirement(40, 2);

    // With RAM offload available the model still runs, but never by splitting
    // across vendors: it degrades to a single device plus host memory.
    let fallback = plan(
        &devices,
        &budgets,
        &req,
        &GpuConfig::default(),
        full_runtime(),
    )
    .unwrap();
    assert_eq!(fallback.strategy, ExecutionStrategy::CpuOffload);
    assert_eq!(fallback.devices.len(), 1, "no cross-vendor split");

    // Without RAM offload there is no plan left, and the reason must name the
    // policy that blocked it.
    let no_offload = RuntimeFacts {
        supports_gpu_offload: true,
        supports_tensor_parallel: false,
        supports_cpu_offload: false,
    };
    let err = plan(&devices, &budgets, &req, &GpuConfig::default(), no_offload).unwrap_err();
    assert!(err.reason.contains("different vendors"), "{}", err.reason);

    // Opting in explicitly allows the split.
    let policy = GpuConfig {
        allow_cross_vendor_model_split: true,
        ..GpuConfig::default()
    };
    let allowed = plan(&devices, &budgets, &req, &policy, full_runtime()).unwrap();
    assert_eq!(allowed.devices.len(), 2);
}

#[test]
fn multi_gpu_disabled_falls_back_to_cpu_offload_rather_than_failing() {
    // A model too large for one card but with a runtime that can keep the rest in
    // RAM must still run when multi-GPU is switched off.
    let (devices, budgets) = two_nvidia();
    let policy = GpuConfig {
        allow_multi_gpu: false,
        ..GpuConfig::default()
    };

    let fallback = plan(
        &devices,
        &budgets,
        &requirement(20, 2),
        &policy,
        full_runtime(),
    )
    .unwrap();
    assert_eq!(fallback.strategy, ExecutionStrategy::CpuOffload);
    assert_eq!(fallback.devices.len(), 1, "one device is all offload needs");
}

#[test]
fn cpu_offload_is_offered_when_vram_is_short_but_ram_offload_is_supported() {
    let (devices, budgets) = two_nvidia();
    let policy = GpuConfig {
        allow_multi_gpu: false,
        ..GpuConfig::default()
    };

    // Larger than any card, but the runtime can keep the rest in RAM.
    let offloaded = plan(
        &devices,
        &budgets,
        &requirement(60, 2),
        &policy,
        full_runtime(),
    )
    .unwrap();
    assert_eq!(offloaded.strategy, ExecutionStrategy::CpuOffload);
    assert!(
        offloaded.reason.contains("slower"),
        "the cost must be stated"
    );
}

#[test]
fn no_plan_is_possible_when_nothing_fits_and_offload_is_unsupported() {
    let (devices, budgets) = two_nvidia();
    let policy = GpuConfig {
        allow_multi_gpu: false,
        ..GpuConfig::default()
    };
    let runtime = RuntimeFacts {
        supports_gpu_offload: true,
        supports_tensor_parallel: false,
        supports_cpu_offload: false,
    };

    let err = plan(&devices, &budgets, &requirement(60, 2), &policy, runtime).unwrap_err();
    assert!(
        err.reason.contains("multi-GPU is disabled"),
        "the reason must name the policy that blocked it: {}",
        err.reason
    );
    assert!(
        err.reason.contains("cannot offload to RAM"),
        "and must say the last fallback is unavailable: {}",
        err.reason
    );
}

#[test]
fn a_machine_with_no_gpu_is_refused_clearly() {
    let err = plan(
        &[],
        &[],
        &requirement(1, 1),
        &GpuConfig::default(),
        full_runtime(),
    )
    .unwrap_err();
    assert!(err.reason.contains("no GPU"), "{}", err.reason);
}

#[test]
fn a_runtime_without_gpu_offload_is_refused_before_any_budget_math() {
    let (devices, budgets) = two_nvidia();
    let runtime = RuntimeFacts {
        supports_gpu_offload: false,
        supports_tensor_parallel: false,
        supports_cpu_offload: false,
    };
    let err = plan(
        &devices,
        &budgets,
        &requirement(1, 1),
        &GpuConfig::default(),
        runtime,
    )
    .unwrap_err();
    assert!(err.reason.contains("cannot offload"), "{}", err.reason);
}

// -------------------------------------------------------------- ledger -----

#[test]
fn a_lease_reserves_bytes_and_release_returns_them() {
    let ledger = GpuLedger::default();
    let usable = 10 * GIB;

    let lease = ledger
        .reserve(ReserveRequest {
            lease_id: "l1",
            device_id: "gpu0",
            bytes: 4 * GIB,
            holder: "model-a",
            usable_bytes: usable,
            now_ms: 1_000,
            ttl_ms: 60_000,
        })
        .unwrap();
    assert_eq!(lease.device_id, "gpu0");
    assert_eq!(ledger.reserved_on("gpu0"), 4 * GIB);

    ledger.release(&lease.lease_id).expect("lease exists");
    assert_eq!(ledger.reserved_on("gpu0"), 0);
}

#[test]
fn over_reserving_is_refused_with_the_numbers_in_the_message() {
    let ledger = GpuLedger::default();
    let usable = 10 * GIB;
    ledger
        .reserve(ReserveRequest {
            lease_id: "l1",
            device_id: "gpu0",
            bytes: 8 * GIB,
            holder: "model-a",
            usable_bytes: usable,
            now_ms: 0,
            ttl_ms: 60_000,
        })
        .unwrap();

    let err = ledger
        .reserve(ReserveRequest {
            lease_id: "l2",
            device_id: "gpu0",
            bytes: 4 * GIB,
            holder: "model-b",
            usable_bytes: usable,
            now_ms: 0,
            ttl_ms: 60_000,
        })
        .unwrap_err();
    assert!(err.0.contains("already reserved"), "{}", err.0);
    assert_eq!(
        ledger.reserved_on("gpu0"),
        8 * GIB,
        "the refused lease must not apply"
    );
}

#[test]
fn leases_are_per_device_so_two_gpus_do_not_share_a_budget() {
    let ledger = GpuLedger::default();
    ledger
        .reserve(ReserveRequest {
            lease_id: "l1",
            device_id: "gpu0",
            bytes: 9 * GIB,
            holder: "a",
            usable_bytes: 10 * GIB,
            now_ms: 0,
            ttl_ms: 60_000,
        })
        .unwrap();
    ledger
        .reserve(ReserveRequest {
            lease_id: "l2",
            device_id: "gpu1",
            bytes: 9 * GIB,
            holder: "b",
            usable_bytes: 10 * GIB,
            now_ms: 0,
            ttl_ms: 60_000,
        })
        .expect("a second device has its own budget");

    assert_eq!(ledger.reserved_on("gpu0"), 9 * GIB);
    assert_eq!(ledger.reserved_on("gpu1"), 9 * GIB);
}

#[test]
fn releasing_an_unknown_lease_is_harmless() {
    let ledger = GpuLedger::default();
    assert!(ledger.release("never-existed").is_none());
    assert_eq!(ledger.reserved_on("gpu0"), 0);
}

#[test]
fn expired_leases_can_be_reaped_so_a_crash_cannot_pin_a_gpu() {
    let ledger = GpuLedger::default();
    ledger
        .reserve(ReserveRequest {
            lease_id: "crashed",
            device_id: "gpu0",
            bytes: 5 * GIB,
            holder: "dead-worker",
            usable_bytes: 10 * GIB,
            now_ms: 1_000,
            ttl_ms: 1_000,
        })
        .unwrap();
    ledger
        .reserve(ReserveRequest {
            lease_id: "alive",
            device_id: "gpu0",
            bytes: 3 * GIB,
            holder: "live-worker",
            usable_bytes: 10 * GIB,
            now_ms: 5_000,
            ttl_ms: 60_000,
        })
        .unwrap();

    // At t=3_000 only the first lease has expired.
    assert_eq!(ledger.expired(3_000).len(), 1);
    assert_eq!(ledger.reap_expired(3_000), 1);
    assert_eq!(
        ledger.reserved_on("gpu0"),
        3 * GIB,
        "the live lease is untouched"
    );
    assert_eq!(ledger.leases().len(), 1);
}
