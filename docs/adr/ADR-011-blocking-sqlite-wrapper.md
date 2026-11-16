> **English** | [中文](ADR-011-blocking-sqlite-wrapper.zh.md)

# ADR-011 — SQLite access through a blocking wrapper, not an async driver

**Status:** Accepted (implementation decision for Task 2)

## Context

Spec §10 asks for SQLite with versioned migrations. The obvious Rust choice,
`sqlx`, is async and its compile-time-checked macros require a live database (or
a checked-in `.sqlx` cache) at build time. Additionally, sqlx 0.9 requires
rustc 1.94, above this project's pinned toolchain.

## Decision

Use `rusqlite` (bundled SQLite, no system dependency) behind a thin async
wrapper:

* one connection guarded by a mutex;
* every statement runs on `tokio::task::spawn_blocking`, so a slow write cannot
  stall the async scheduler;
* `journal_mode = WAL` (readers never block the writer),
  `synchronous = NORMAL`, `foreign_keys = ON`, and a 5 s `busy_timeout`;
* migrations are plain `.sql` files embedded with `include_str!`, applied in a
  transaction together with their version row, and recorded in
  `schema_migrations`.

## Consequences

- The schema is real SQL, reviewable in a diff, with no macro-generated queries.
- A database written by a newer build is **refused** (`MigrationError::TooNew`)
  instead of being silently opened and corrupted.
- `Database::call` is `async fn` and always offloads; `call_blocking` exists only
  for startup checks and tests, and is documented as such.
- Only one writer exists at a time. That is fine for a local registry whose
  writes are model metadata, settings and task records; if profiling later shows
  contention, a connection pool is a local change behind `Database`.
- No build-time database is needed, so a clean `cargo build` works on a machine
  that has never run the product.
