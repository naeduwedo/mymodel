# ADR-007 — Core and inference workers are separate processes

**Status:** Accepted (spec §16, ADR-007)

## Context

Model loading and inference involve drivers and multi-gigabyte allocations;
crashes there are realistic and must not take down the manager or lose the
user's queue.

## Decision

Core (policy, ledger, API) and runtime workers (inference) are separate
processes. Workers are supervised: heartbeats, captured stdout/stderr,
cancellation and restart. A UI disconnecting does not kill workers unless
configured (`keep_workers_after_ui_exit`).

## Consequences

- The worker process exists from Task 1 (`gguf-worker`), even though it has no
  adapter yet and exits with a clear "unsupported" code.
- Inter-process protocol design matters from the start; it lands in Task 2.
- Worker crash recovery is a supported scenario, not an exceptional one.
