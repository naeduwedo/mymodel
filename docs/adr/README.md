> **English** | [中文](README.zh.md)

# Architecture decision records

Format: context → decision → consequences. The first seven are transcribed from
spec §16 (they are normative); the rest are decisions this repository made
while implementing Task 1 and are still binding.

| ADR | Title | Status |
| --- | --- | --- |
| [001](ADR-001-rust-core-native-uis.md) | Rust core with three native UIs | Accepted (spec) |
| [002](ADR-002-gguf-first.md) | GGUF-first, not GGUF-only | Accepted (spec) |
| [003](ADR-003-runtime-adapter.md) | Runtime adapters, no home-grown kernels | Accepted (spec) |
| [004](ADR-004-capability-driven-multi-gpu.md) | Capability-driven multi-GPU | Accepted (spec) |
| [005](ADR-005-runtime-managed-paging.md) | Runtime-managed paging before engine-managed paging | Accepted (spec) |
| [006](ADR-006-nvme-is-a-cold-tier.md) | NVMe is a cold tier | Accepted (spec) |
| [007](ADR-007-core-worker-isolation.md) | Core and inference workers are separate processes | Accepted (spec) |
| [008](ADR-008-bounded-typed-event-bus.md) | Bounded, typed, replayable event bus | Accepted |
| [009](ADR-009-strict-typed-configuration.md) | Strict, typed configuration with path containment | Accepted |
| [010](ADR-010-unsafe-windows-pipe-acl.md) | Narrow `unsafe` exception for the Windows pipe DACL | Accepted |
| [011](ADR-011-blocking-sqlite-wrapper.md) | SQLite through a blocking wrapper, not an async driver | Accepted |
| [012](ADR-012-core-owns-presentation.md) | The core owns presentation; shells only render | Accepted |
| [013](ADR-013-cli-based-hardware-probes.md) | Probe GPUs through vendor CLIs, not vendor FFI | Accepted |
| [014](ADR-014-defensive-gguf-parsing.md) | GGUF parsing: bounded prefix read, no mmap | Accepted |
| [015](ADR-015-download-integrity.md) | Download integrity: verify before ready, commit atomically | Accepted |
| [016](ADR-016-llama-cpp-http-server.md) | llama.cpp integration through its HTTP server, not FFI | Accepted |
| [017](ADR-017-per-device-budgets-and-leases.md) | Per-device budgets and expiring leases, never a summed pool | Accepted |
| [018](ADR-018-modality-status-not-flags.md) | Modalities are declared with a status and a reason, never a flag | Accepted |
| [019](ADR-019-packaging-ships-binaries-only.md) | Packaging ships binaries and recipes, never user data or runtimes | Accepted |
| [020](ADR-020-openai-gateway-relays-the-core-stream.md) | The OpenAI gateway relays the core's stream instead of loading models | Accepted |

## Writing a new ADR

Create `ADR-NNN-short-title.md` with `Status`, `Context`, `Decision`,
`Consequences` sections, add it to the table above, and link it from the code
it governs. An ADR supersedes another by changing that file's status to
`Superseded by ADR-NNN` rather than rewriting history.

## Adding `unsafe` code

The workspace forbids `unsafe_code`. A crate that needs it must (a) add an ADR
explaining why no safe alternative exists, (b) narrow the exception to that
crate only, (c) re-establish `#![deny(unsafe_code)]` at the crate root and allow
it per module, and (d) update the CI `deny-unsafe` job so the exception is
visible in review.

Current exceptions: [ADR-010](ADR-010-unsafe-windows-pipe-acl.md) only.