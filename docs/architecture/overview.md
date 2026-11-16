> **English** | [中文](overview.zh.md)

# Architecture overview

> Normative source: [`GGUF-AI-Engine-Engineering-Spec.md`](../../GGUF-AI-Engine-Engineering-Spec.md) §3.
> This document explains how the code in this repository realises it, and
> records what is deliberately **not** built yet.
>
> Since Task 2, the IPC transport, the gRPC services and the SQLite layer are
> implemented; they are described in
> [`ipc-and-storage.md`](ipc-and-storage.md).

## Process model (spec §3.1)

```text
Native Desktop UI (WinUI 3 / GTK4 / SwiftUI)
        │  gRPC over named pipe (Windows) / Unix socket (Linux, macOS)
        ▼
Rust Core Daemon  — gguf-core
   ├── config        typed schema, defaults, validation, path containment
   ├── event-bus     bounded, typed, replayable events to every UI
   ├── engine-core   lifecycle state machine, health, single-instance lock
   ├── database      SQLite + versioned migrations            (Task 2)
   ├── api-server    REST + OpenAI-compatible gateway         (Task 2)
   ├── hardware      CPU/RAM/storage/GPU probes               (Task 4)
   ├── gguf          bounds-checked GGUF parser               (Task 5)
   ├── model-hub     Hugging Face search/download             (Task 5)
   ├── gpu-manager   topology, budgets, leases                (Task 7)
   ├── memory-manager VRAM/RAM/NVMe orchestration             (Task 7)
   └── runtime-manager / runtime-llama-cpp                    (Task 6)
        │
        └── Runtime Worker processes — gguf-worker (ADR-007)
```

The core owns **policy and the resource ledger**; workers own **inference**.
A UI crash must never terminate a model worker unless the user asked for that
(`[core] keep_workers_after_ui_exit`).

## Layers

| Layer | Crate | Rule |
| --- | --- | --- |
| Presentation | `platforms/` (planned) | No business logic; talks only to the core. |
| Transport | `crates/protocol`, `crates/api-server` | Versioned, capability-negotiated. |
| Policy | `crates/engine-core`, `gpu-manager`, `memory-manager` | Single owner of every budget and lease. |
| Domain | `crates/gguf`, `model-hub`, `hardware` | Pure, testable, no I/O policy. |
| Infrastructure | `crates/database`, `telemetry`, `config` | Replaceable, no domain decisions. |

Dependency direction is strictly downward. `engine-core` depends on
`config` and `event-bus` only; nothing depends on `apps/`.

## What Task 1 actually implements

Verify any claim with `cargo run -p gguf-engine-cli -- doctor`.

### Implemented and tested

- **Configuration** (`crates/config`): full `config.toml` schema from spec §11,
  strict parsing (unknown keys rejected), per-field range and cross-field
  validation, and path containment — a configured directory that resolves
  outside the data root is rejected instead of silently used.
- **Paths** (`crates/config::paths`): platform data roots (spec §15), lexical
  normalisation, idempotent directory creation.
- **Event bus** (`crates/event-bus`): bounded broadcast with sequence numbers,
  UTC timestamps, correlation ids, a bounded replay ring for reconnecting UIs,
  and explicit backpressure/lag accounting instead of unbounded memory.
- **Lifecycle** (`crates/engine-core::lifecycle`): the state machine from spec
  §3.3 with validated transitions; every transition is published to the bus.
- **Health** (`crates/engine-core::health`): per-component reports, a
  machine-readable snapshot, and `is_healthy()` that refuses to call a starting
  core healthy.
- **Single-instance lock** (`crates/engine-core::lock`): exclusive OS file
  handle; released automatically by the OS if the process dies, so a crash
  cannot leave a stale lock behind.
- **CLI** (`apps/cli`): `doctor`, `config`, `serve`, `version`, with `--json`
  everywhere and meaningful exit codes.
- **Daemon** (`apps/core-daemon`): configuration → directories → lock →
  lifecycle → health tick loop, graceful Ctrl+C shutdown.
- **Telemetry** (`crates/telemetry`): env-filtered tracing to stderr, pretty or
  JSON, with a logged fallback when the filter directive is invalid.
- **CI**: fmt, clippy (`-D warnings`) and tests on Linux, Windows and macOS
  plus an MSRV check and an `unsafe` guard.

### Deliberately not implemented

| Capability | Why | Where it is stated |
| --- | --- | --- |
| gRPC over named pipe / Unix socket | **done (Task 2)** | `crates/ipc`, e2e tests in `tests/tests/ipc_e2e.rs` |
| SQLite schema & migrations | **done (Task 2)** | `crates/database`, `migrations/0001_init.sql` |
| HTTP / OpenAI-compatible API | next | `api-server` crate `STATUS = Planned`; no listener is started |
| Hardware / GPU probes | **done (Task 4)** — see [hardware-inventory.md](hardware-inventory.md) | `hardware` crate `STATUS = Planned` |
| GGUF parser, Hub download | **done (Task 5)** — see [gguf-and-model-hub.md](gguf-and-model-hub.md) | `gguf`, `model-hub` `STATUS = Planned` |
| llama.cpp adapter | Task 6 | `runtime-llama-cpp` `STATUS = Planned`; `gguf-worker` exits 4 |
| Non-LLM runtimes (image, video, speech) | **done** — see [multimodal-runtimes.md](multimodal-runtimes.md) |
| Multi-GPU planning, paging | **done (Task 7)** — see [multi-gpu-and-memory.md](multi-gpu-and-memory.md) | `gpu-manager`, `memory-manager` `STATUS = Planned` |
| Native UI shells | Task 3 | `platforms/` holds only READMEs |

The daemon therefore runs in `degraded` and not `ready`: a UI must be able to
show the truth, and the health report lists `ipc` and `database` as unhealthy
components with reasons.

## Truthfulness mechanisms

Spec rule 4 ("no fake features") is enforced structurally, not by discipline:

1. `gguf_engine_protocol::supported_features()` is the single list UIs read, and
   a test asserts unimplemented features are absent from it.
2. Planned crates expose `STATUS = Status::Planned`.
3. `gguf doctor` prints `planned` work next to the checks that pass.
4. The daemon's health report marks unimplemented components unhealthy rather
   than omitting them.

## Failure semantics

| Failure | Behaviour |
| --- | --- |
| Invalid config | Start-up fails with `field: reason` for every error, not just the first. |
| Path outside the data root | Rejected (`PathEscape`), never silently accepted. |
| Second core instance | Exit code 3 with the owning pid in the message. |
| Slow UI subscriber | Events are dropped and counted; the publisher is never blocked. |
| Console cannot receive logs | Logs go to stderr only, so a closed stdout never changes the exit code of a successful command. |