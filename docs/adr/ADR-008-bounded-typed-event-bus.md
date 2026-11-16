> **English** | [中文](ADR-008-bounded-typed-event-bus.zh.md)

# ADR-008 — Bounded, typed, replayable event bus

**Status:** Accepted (implementation decision for Task 1)

## Context

Every UI needs a live stream of state changes (lifecycle, workers, downloads,
memory pressure). Naive designs — unbounded channels or a shared mutable
snapshot — either grow without limit when a UI is slow, or lose the ordering
needed to render a coherent state.

## Decision

`gguf-engine-event-bus` provides:

- **typed payloads** (`EventPayload`), never stringly-typed messages;
- a **bounded** per-subscriber queue: a slow UI loses events, and the loss is
  counted on both sides (`BusStats::dropped`, `EventSubscriber::lagged`),
  instead of blocking the inference path or growing memory;
- a **monotonic `seq`** plus a bounded replay ring so a reconnecting UI can
  resume with `replay_after(seq)` and detect gaps;
- UTC timestamps and correlation ids for task-scoped events.

## Consequences

- Publishers never block on a UI. They may receive `Backpressure`, which is
  logged and counted, not propagated as a fatal error.
- The UI contract is "state is authoritative, events are the delta": a client
  that misses events can re-read health/state and continue.
- The replay ring is memory-bounded (`replay_limit`), so it is a convenience,
  not a durability mechanism; durable history is a database concern (Task 2).
