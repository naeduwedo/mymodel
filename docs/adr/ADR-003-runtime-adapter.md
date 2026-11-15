# ADR-003 — Runtime adapters, no home-grown kernels

**Status:** Accepted (spec §16, ADR-003)

## Context

Writing CUDA/ROCm/Metal kernels and a transformer engine is a multi-year
effort with continuous hardware maintenance, and it is not this product's
differentiator.

## Decision

Integrate mature runtimes (llama.cpp first) through adapters implementing a
common trait: capability probe, memory estimate, load, infer, unload. No
inference kernel is implemented in this repository.

## Consequences

- Hardware support follows the runtime's support, and we must report that
  honestly per adapter and per version.
- Adapter version skew is a real risk: each adapter is versioned, tested and
  released separately.
- Kernel-level optimisations are contributed upstream where they belong.
