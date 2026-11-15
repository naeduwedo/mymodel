# ADR-005 — Runtime-managed paging before engine-managed paging

**Status:** Accepted (spec §16, ADR-005)

## Context

Two managers moving the same tensor — one inside the runtime, one in the
engine — is a correctness bug waiting to happen, and can corrupt inference.

## Decision

Prefer runtime-native mmap, layer offload and expert caching. Enable
engine-managed tensor paging only for runtimes that expose tensor lifetime,
transfer and synchronisation primitives. When the engine manages a tensor it
owns the budget; when the runtime does, the engine only budgets, configures,
monitors and never duplicates that accounting.

## Consequences

- `allow_engine_managed_paging` defaults to false and is gated per runtime.
- Memory reports must distinguish "runtime-managed" from "engine-managed" so
  the numbers in the Memory Manager page mean the same thing everywhere.
