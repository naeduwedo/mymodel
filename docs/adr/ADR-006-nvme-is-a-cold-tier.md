# ADR-006 — NVMe is a cold tier

**Status:** Accepted (spec §16, ADR-006)

## Context

NVMe bandwidth and, crucially, latency are orders of magnitude away from
VRAM. Treating NVMe as an extension of VRAM produces unpredictable decode
times and wears drives.

## Decision

NVMe stores model files and cold data, with bounded, cancellable, measured
prefetch. It is never presented as equivalent memory. The operating system's
swap/pagefile is never used as a model paging mechanism.

## Consequences

- Memory estimates must be explainable per tier; a model that only fits "with
  NVMe" is reported as such, with the expected slowdown stated.
- Prefetch has a hard byte budget (`max_prefetch_bytes`) and reports accuracy.
- "It works" is never claimed for configurations that rely on thrashing.
