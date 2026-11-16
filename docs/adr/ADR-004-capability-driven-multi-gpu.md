> **English** | [中文](ADR-004-capability-driven-multi-gpu.zh.md)

# ADR-004 — Capability-driven multi-GPU

**Status:** Accepted (spec §16, ADR-004)

## Context

Multi-GPU support is a headline feature, but vendors and runtimes differ
wildly: P2P availability, tensor parallel support, unified versus discrete
memory, and driver behaviour.

## Decision

Every runtime declares capabilities and their status
(`Supported / Unsupported / Experimental / Unknown`). The planner only proposes
a strategy the runtime declares. P2P is reported as detected,
driver-supported or benchmark-verified — never inferred from a device list.

## Consequences

- Unknown is a first-class state; it is never coerced into true or false.
- The UI cannot offer Tensor Parallel for a runtime that has not proven it.
- Users get an explanation ("not supported by this runtime") instead of a
  failed load five minutes later.
