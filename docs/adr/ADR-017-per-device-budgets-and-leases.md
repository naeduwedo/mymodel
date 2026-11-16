> **English** | [中文](ADR-017-per-device-budgets-and-leases.zh.md)

# ADR-017 - Per-device budgets and expiring leases, never a summed pool

**Status:** Accepted (implementation decision for Task 7)

## Context

Spec 5.4 forbids presenting the sum of several GPUs' VRAM as one pool, and
requires every allocation to carry a budget, a reserve and a release. Two
tempting shortcuts are wrong here:

- adding all VRAM and planning against the total, which produces plans that
  no runtime can execute (a 20 GiB tensor cannot straddle two cards unless the
  runtime splits by layer);
- tracking allocations without a deadline, so a crashed worker keeps its
  reservation forever and the GPU is unusable until the core restarts.

## Decision

1. **Budgets are per device.** `usable = observed_free - engine_reserve -
   safety_margin`, computed independently for each card. Unknown free VRAM
   yields zero usable bytes rather than an optimistic guess.
2. **The planner is a pure function** of (devices, budgets, requirement,
   policy, runtime facts). It returns a strategy only when that strategy is
   executable on this machine with this runtime.
3. **Every reservation is a lease with a TTL.** Expired leases can be reaped,
   so a crash cannot pin a device.
4. **Multi-GPU policy blocks the split, not the load.** When `allow_multi_gpu`
   is false, or the cards are from different vendors without an opt-in, the
   planner falls back to CPU offload instead of failing outright: the model may
   still be runnable on one card plus host RAM.

## Consequences

- The planner is exhaustively testable without a GPU; 17 tests cover the
  strategy ladder on this project's real 4090 + 3060 configuration.
- A user sees *why*: every plan and every refusal carries a sentence naming the
  numbers and the policy that produced it.
- Tensor parallel is only proposed when the runtime declares it, because it is
  the strategy most likely to fail at load time.

## Verification

Real machine (RTX 4090 + RTX 3060, 22.0 + 11.8 GiB free):

    8 GiB  -> single_gpu      (fits the 4090: 19.5 GiB usable)
    20 GiB -> layer_split     (19.5 + 2.2 GiB across both)
    60 GiB -> cpu_offload     (28.8 GiB VRAM total, rest in RAM, stated as slower)
