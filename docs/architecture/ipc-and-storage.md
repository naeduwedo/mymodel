> **English** | [中文](ipc-and-storage.zh.md)

# Architecture — IPC, database and the Task 2 core

> Normative source: [`GGUF-AI-Engine-Engineering-Spec.md`](../../GGUF-AI-Engine-Engineering-Spec.md) §3, §9–§10.
> This document explains how the code realises §3.2 (communication), §3.3
> (lifecycle) and §10 (database), and what is deliberately not built yet.

## Transport (spec §3.2)

```text
CLI / native UI
   │  gRPC over http/2
   ▼
┌────────────────────────────── gguf-core ─────────────────────────────┐
│ Windows: \\.\pipe\gguf-ai-engine-<hash>   DACL = user + SYSTEM       │
│ Unix:    <data_root>/run/core.sock          directory mode = 0700    │
│                                                                      │
│  HealthService   GetHealth / Ping                                    │
│  EventService    Subscribe(after_seq) → stream (replay, then live)   │
└──────────────────────────────────────────────────────────────────────┘
```

Why not TCP: a local socket cannot be reached from another machine even when a
firewall is misconfigured, and its access control is enforced by the OS rather
than by a token the core would have to distribute.

### Windows specifics

- One pipe instance accepts one client, so the server **always keeps a spare
  instance listening**. Creating the next instance only after a client connects
  leaves a window where a client gets `ERROR_FILE_NOT_FOUND` — indistinguishable
  from "the core is not running".
- The security descriptor grants the current user and SYSTEM, nothing else
  (ADR-010). This requires a scoped `unsafe` exception; the workspace-wide
  `unsafe` ban otherwise still applies.
- tonic has no pipe support, so `crates/ipc/src/transport_windows.rs` provides a
  `PipeStream` (server side, implementing tonic's `Connected`) and a
  `PipeConnector` (client side, a `tower::Service<Uri>`).

### Unix specifics

- The socket lives in `<data_root>/run/`, created `0700`, so another local user
  cannot connect by path.
- A stale socket from a crashed core is removed at bind time; the single-instance
  lock guarantees no live core owns that path.

### Endpoint naming

`gguf_engine_protocol::ipc_endpoint(data_root)` derives the endpoint from the
data root: a stable FNV-1a tag for the pipe name (pipe names are limited to 256
characters), or `run/core.sock` inside the data root. Two installations never
collide, and a test suite does not disturb a developer's running core.

## Services

| Service | Methods | Status |
| --- | --- | --- |
| `HealthService` | `GetHealth`, `Ping` | implemented |
| `EventService` | `Subscribe` (server streaming) | implemented |
| `HardwareService` | `GetHardware` | schema only → `UNIMPLEMENTED` |
| `ModelService`, `RuntimeService`, `InferenceService`, `TaskService`, `GpuService`, `MemoryService` | — | schema only |

Only implemented services are registered with the router. A client calling an
unimplemented one gets gRPC `UNIMPLEMENTED`, which is the honest answer and is
asserted by a test (`crates/ipc/tests/services.rs`).

### Health reply

The health report deliberately mixes facts with gaps:

```text
state: ready
components:
  ipc                   healthy
  database              healthy   "schema v1, sqlite 3.50.2"
  event-bus             healthy
  hardware_inventory    unhealthy "Task 4: no CPU/GPU probe is implemented yet"
  runtime_llama_cpp     unhealthy "Task 6: no runtime adapter is installed"
  ...
```

`healthy` (the aggregate flag) is true only when the state is `ready`/`degraded`
**and** every component is healthy. An unfinished subsystem therefore cannot be
mistaken for a working one by a UI that only reads the aggregate.

### Event stream semantics

- `Subscribe(after_seq)` first replays the bounded ring for that data root, then
  continues live; duplicates are filtered by sequence number.
- A slow client loses events rather than growing core memory, and the loss is
  counted (`BusStats.dropped`, `EventSubscriber::lagged`).
- State is authoritative, events are the delta: a client that missed events can
  re-read `GetHealth`.

## Lifecycle (spec §3.3)

```text
Stopped ──▶ Starting ──▶ Ready ⇄ Degraded ──▶ ShuttingDown ──▶ Stopped
```

The daemon's startup is ordered so a partial failure leaves nothing open:

1. resolve data root, load and validate `config.toml`;
2. create the directory tree;
3. take the single-instance lock (exit code 3 if a core is already running);
4. open and migrate SQLite;
5. bind the IPC endpoint (exit code 4 if it cannot be bound);
6. `Stopped → Starting` before binding, then `→ Ready` and publish the change.

Shutdown reverses this: `ShuttingDown` → stop the listener (with a 5 s grace
period, then abort) → `Stopped` → release the lock.

The grace period matters: a streaming client can hold a request open forever,
and a UI that forgets to disconnect must not be able to pin the core.

## Database (spec §10)

`migrations/0001_init.sql` creates all 19 tables the specification lists.
Conventions enforced by tests:

- capacities are `INTEGER` bytes — a `REAL` byte column fails
  `capacity_columns_are_integers_not_floats`;
- timestamps are `INTEGER` UTC epoch milliseconds;
- `STRICT` tables, so a wrong type is an error rather than a silent coercion;
- foreign keys are declared **and enforced** (`PRAGMA foreign_keys = ON`).

Migrations are forward-only, recorded in `schema_migrations`, and applied inside
a transaction with their version row. A database written by a newer build is
refused (`MigrationError::TooNew`) rather than opened and corrupted.

Access model (ADR-011): one `rusqlite` connection behind a mutex, every statement
on `spawn_blocking`, WAL + `synchronous=NORMAL` + `busy_timeout`.

## Security defaults (spec §9.3)

| Concern | Implementation |
| --- | --- |
| Local-only API | no TCP listener at all; the IPC transport is a pipe/socket |
| Peer isolation | pipe DACL (Windows) / `0700` socket directory (Unix) |
| Token comparison | constant-time `gguf_engine_security::Token`, redacted `Debug` |
| Path containment | `AppPaths` rejects anything resolving outside the data root |
| Secret logging | tokens are never formatted into logs; `Debug` prints only the length |
| Untrusted repository content | nothing from a model repo is executed; there is no downloader yet |

## Testing strategy for this layer

| Layer | Test | Note |
| --- | --- | --- |
| Schema | `crates/database/src/migrations.rs` | table list, column types, upgrade and refusal paths |
| Services | `crates/ipc/tests/services.rs` | real gRPC over loopback TCP, portable across CI |
| Transport | `tests/tests/ipc_e2e.rs` | **real binaries** over the **real pipe/socket** |
| Honesty | `features()` + `UNIMPLEMENTED` assertions | a service cannot be advertised before it exists |
| Docs | `tests/tests/docs_consistency.rs` | example config and crate status stay in sync |

The loopback TCP path exists only for tests and future remote workers; it binds
`127.0.0.1` and is never used by the daemon.
