> **English** | [中文](README.zh.md)

# Documentation

Most documents here exist in two editions: the English file (`X.md`) and its
Chinese translation (`X.zh.md`), kept in step, with a toggle at the top of each.
The exception is the build and usage manual, which was written in Chinese only;
an English edition of it does not exist yet.

## Start here

| Document | What it answers |
| --- | --- |
| [Build and usage manual](BUILD-AND-USAGE.md) (Chinese only) | How to compile it, configure it and use every modality. |
| [Architecture overview](architecture/overview.md) | How the pieces fit together. |
| [Task status](architecture/task-status.md) | What is finished, with the evidence. |

## Architecture

| Document | Scope |
| --- | --- |
| [overview.md](architecture/overview.md) | Layering, data flow, process model |
| [ipc-and-storage.md](architecture/ipc-and-storage.md) | gRPC transport, SQLite, migrations |
| [gguf-and-model-hub.md](architecture/gguf-and-model-hub.md) | GGUF parsing, Hub download and verification |
| [hardware-inventory.md](architecture/hardware-inventory.md) | CPU, memory, storage and GPU probing |
| [multi-gpu-and-memory.md](architecture/multi-gpu-and-memory.md) | Per-device budgets, leases, tiering |
| [multimodal-runtimes.md](architecture/multimodal-runtimes.md) | The four non-LLM runtimes, and what was verified |
| [native-shells.md](architecture/native-shells.md) | The Windows, Linux and macOS shells |
| [task-status.md](architecture/task-status.md) | Per-task completion record |

## Decision records

[adr/README.md](adr/README.md) indexes ADR-001 to ADR-020. Each records a decision
that is expensive to reverse, and the reasoning behind it.

## Specifications

- [GGUF-AI-Engine-Engineering-Spec.md](../GGUF-AI-Engine-Engineering-Spec.md) — the
  normative product specification (Chinese; it predates this documentation set).