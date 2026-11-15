# Multi-GPU scheduling and the memory orchestrator (Task 7)

> Normative source: spec §5 (multi-GPU) and §6 (memory orchestrator).

## Two rules that shape the whole design

**1. VRAM is budgeted per device, never summed.**

    usable(device) = observed_free - engine_reserve - safety_margin

`total_usable()` exists for display only, and every place that shows it says so.
A 20 GiB tensor cannot straddle two cards unless the runtime splits by layer, so
planning against a summed pool produces plans that fail at load time.

**2. Every reservation is a lease with a deadline.**

A worker that crashes must not pin a GPU forever. Leases carry a TTL and can be
reaped, so recovery is automatic rather than "restart the core".

## The planner is a pure function

    plan(devices, budgets, requirement, policy, runtime_facts) -> Plan | PlanError

It takes no locks, touches no GPU and spawns nothing, so it is exhaustively
testable. The strategy ladder, in order:

| # | Strategy | Condition |
| --- | --- | --- |
| 1 | `single_gpu` | one card's usable budget covers the whole requirement |
| 2 | `tensor_parallel` | total fits **and** the runtime declares TP **and** two same-vendor cards exist |
| 3 | `layer_split` | total fits across cards of one vendor (or opt-in cross-vendor) |
| 4 | `cpu_offload` | one card plus host RAM, when the runtime can offload |
| 5 | error | nothing fits, with a sentence naming the numbers and the policy |

Stability comes before throughput: a single card is always preferred, and tensor
parallel is last among the multi-GPU options because it is the one most likely
to fail in practice.

### Policy blocks the split, not the load

When `allow_multi_gpu = false`, or the cards are from different vendors without
`allow_cross_vendor_model_split`, the planner does **not** fail immediately: it
falls back to CPU offload, because the model may still run on one card plus RAM.
Only when that fallback is also unavailable does it return an error, and the
error names both the blocking policy and the missing fallback.

This was found by a test that expected an error and got a plan. The test was
right and the planner was wrong.

## Memory tiers

| Tier | Contents | Addressable? |
| --- | --- | --- |
| L0 `gpu_vram` | weights, KV cache, activations, hot experts | yes, per device |
| L1 `host_ram` | CPU weights, page cache, staging | yes |
| L2 `nvme` | model files, cold data | **no** — a cold tier (ADR-006) |

`Tier::is_addressable_memory()` returns false for NVMe, and the CLI prints
`cold` for it. There is no code path that treats NVMe as memory, and the OS swap
file is never used as a model paging mechanism.

### Pressure and prefetch

    nominal  -> available > reserve * 1.5
    elevated -> reserve < available <= reserve * 1.5
    critical -> available <= reserve

Prefetch is capped (`max_prefetch_bytes`) and **stops entirely** under pressure:
it competes with the OS for the reserve, so it is the first thing to cut. A
request larger than the remaining headroom is truncated, not refused.

## What is verified, and how

17 planner/lease tests plus 8 memory tests, using this machine's real
configuration (RTX 4090 24 GiB + RTX 3060 12 GiB, reserves 2 GiB + 0.5 GiB margin):

| Model | Result |
| --- | --- |
| 8 GiB | `single_gpu` on the 4090 (19.5 GiB usable) |
| 20 GiB | `layer_split`, 19.5 + 2.2 GiB |
| 60 GiB | `cpu_offload`, stated as slower |
| 60 GiB, no RAM offload | error naming VRAM shortfall |

`gguf plan` and `gguf memory` run these against live hardware; both are shown in
[task-status.md](task-status.md).

## Not implemented

| Capability | Why it is absent |
| --- | --- |
| Performance benchmark | needs a fixed model, prompt set and repetition count; belongs with Task 9 validation so the numbers are comparable |
| P2P bandwidth measurement | `p2p_tested` stays `Unsupported`; measuring requires a benchmark, not a probe |
| Engine-managed tensor paging | gated off by ADR-005: llama.cpp owns its memory |
| Planner wired into `gguf chat` | the adapter takes `--n-gpu-layers` directly; using a plan is follow-up work |
