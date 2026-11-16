# GGUF AI Engine

Cross-platform local GGUF model manager and inference platform.

- **Product spec:** [GGUF-AI-Engine-Engineering-Spec.md](GGUF-AI-Engine-Engineering-Spec.md) — the project's normative specification.
- **Implementation status:** [docs/architecture/task-status.md](docs/architecture/task-status.md)
- **Architecture:** [docs/architecture/overview.md](docs/architecture/overview.md)
- **Decision records:** [docs/adr/](docs/adr/README.md)

## Documentation

- **[Build and usage manual](docs/BUILD-AND-USAGE.md)** — how to compile it, configure it,
  and use every modality. Chinese edition; the English material is the README plus
  [docs/architecture/](docs/architecture/).
- [Architecture](docs/architecture/overview.md) — layering, data flow, and the per-area notes.
- [Decision records](docs/adr/) — why the awkward parts are the way they are.
- [Task status](docs/architecture/task-status.md) — what is finished, with the evidence.


## Status

| Phase | State |
| --- | --- |
| Task 1 — Repository bootstrap | **done** (workspace, config, event bus, lifecycle, CLI, daemon, CI, docs) |
| Task 2 — Protocol + core | **done** (protobuf/gRPC, named-pipe & Unix-socket IPC, health, event stream, SQLite migrations) |
| Task 3 — Native UI shells | **partial** (Windows shell implemented + tested; Linux written, unverified; macOS planned) |
| Task 4 — Hardware inventory | **done** (CPU/RAM/storage, NVIDIA verified on real 2-GPU machine, AMD experimental, Apple unverified) |
| Task 5 — GGUF + model hub | **done** (defensive parser, Hub search/download/verify/resume, manifest, registry) |
| Task 6 — llama.cpp adapter | planned |
| Task 7 — Multi-GPU + memory | planned |

Nothing is reported as "supported" until it has a real implementation behind it
(spec §18). `gguf doctor` prints the same feature list the UIs use, and every
unimplemented capability is marked `planned` in code *and* in the UI.

## Requirements

- Rust **stable ≥ 1.93** (`rust-toolchain.toml` pins the channel; CI checks the
  declared MSRV too)
- Windows 10/11, Linux, or macOS
- No GPU, driver or runtime is required for Task 1 — hardware probes arrive in
  Task 4.

## Quick start

```bash
cargo build --workspace          # build every crate
cargo test --workspace           # unit + integration + binary end-to-end tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Command line

```bash
# run the core daemon in the foreground (Ctrl+C to stop)
cargo run -p gguf-engine-daemon -- --data-dir ./data

# in another shell: diagnostics, including a live probe of the running core
cargo run -p gguf-engine-cli -- doctor --data-dir ./data

# effective configuration, resolved directories and validation findings
cargo run -p gguf-engine-cli -- config --data-dir ./data
cargo run -p gguf-engine-cli -- config --defaults --json
```

`doctor` exits non-zero only when something is actually wrong. A core that is
not running is reported as such, not as a failure.

### Talking to the core directly

The core speaks gRPC over a local transport (named pipe on Windows, Unix socket
elsewhere). Any gRPC client can use it with the schema in [`proto/`](proto/):

```bash
# health, lifecycle state and the capability matrix
grpcurl -plaintext -unix ./data/run/core.sock localhost:1 gguf.v1.HealthService/GetHealth
```

The Rust client is `gguf_engine_ipc::CoreClient` (`connect`, `health`, `ping`,
`subscribe`).

Both binaries also accept `--data-dir <DIR>` (or `GGUF_AI_ENGINE_DATA_DIR`) so
data never has to live in the platform default location during development.

### Data directories (spec §15)

| Platform | Root |
| --- | --- |
| Windows | `%LOCALAPPDATA%\GGUF-AI-Engine\` |
| Linux | `~/.local/share/gguf-ai-engine/` |
| macOS | `~/Library/Application Support/GGUF AI Engine/` |

`models/`, `cache/`, `cache/downloads/`, `runtimes/`, `logs/`, `database/` are
created on first start. Configured paths are rejected if they resolve outside
the root, so a config file can never redirect a write outside the application
directory.

## Workspace layout

```text
apps/core-daemon        gguf-core      core daemon (lifecycle, health, lock)
apps/cli                gguf           command line client
apps/runtime-worker     gguf-worker    isolated runtime worker (no adapter yet)
crates/config           typed config, defaults, validation, path containment
crates/event-bus        bounded typed event bus with replay
crates/engine-core      lifecycle state machine, health, single-instance lock
crates/protocol         protobuf/gRPC codegen, value types, feature negotiation
crates/database         SQLite schema, versioned migrations, repositories
crates/security         pipe DACL / socket permissions, constant-time tokens
crates/ipc              local gRPC transport: server, client, services
crates/telemetry        tracing setup (log format, env filter)
crates/<others>         planned modules, each documenting its invariants
proto/                  protobuf schema for every service (implemented + future)
migrations/             versioned SQL migrations
tests/                  workspace-level integration and binary tests
```

The full planned layout (proto/, platforms/, packaging/, migrations/) is
created in the repository so paths are stable; empty directories carry a
`README.md` describing what will live there and in which task.

## Design rules that must not regress

1. Unimplemented capabilities are labelled `planned`/`unsupported` — never a
   working-looking button.
2. No inference kernel is written here; runtimes are integrated through
   adapters (ADR-003).
3. Every GPU/RAM/NVMe allocation goes through a budget with reserve and a
   releasing lease.
4. NVMe is a cold tier, never equivalent VRAM (ADR-006); the OS swap file is
   not a model paging mechanism.
5. Nothing from a model repository is executed (spec rule 8).
6. External processes are observable, cancellable and restartable.
7. The IPC surface is local-only: a pipe with a user-only DACL, or a socket in a
   `0700` directory. `unsafe` is allowed in exactly two Windows modules
   ([ADR-010](docs/adr/ADR-010-unsafe-windows-pipe-acl.md)).

## License

Apache-2.0 — see [LICENSE](LICENSE).