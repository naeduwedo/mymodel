> **English** | [中文](ADR-015-download-integrity.zh.md)

# ADR-015 — Download integrity: verify before ready, commit atomically

**Status:** Accepted (implementation decision for Task 5)

## Context

Spec §7.3 requires ETag / SHA-256 verification, resume, atomic writes and
"download completes → verify → register". A half-downloaded 40 GB file that looks
ready is worse than a missing one: the failure surfaces at inference time, far
from where it was caused.

## Decision

The downloader enforces a strict order:

1. **Pre-check** — the destination path is contained (no `..`, no absolute path),
   before any byte moves.
2. **Stage** into `<name>.part` next to the destination, resuming via an HTTP
   `Range` request when a partial file exists.
3. **Verify** — the staged file must match the SHA-256 the Hub published.
4. **Commit** — an atomic rename moves it into place, so a crash cannot leave a
   plausible-looking short file.

Two integrity rules that are easy to get wrong and are enforced by tests:

- a file with **no published hash** is refused (`Unverifiable`) rather than marked
  ready on faith;
- a server that **ignores `Range`** answers `200` instead of `206`, so the client
  restarts the staging file instead of appending a whole file to its own prefix.

Only a verified file is written to the registry as `ready` (spec §7.3).

## Consequences

- `model-hub` is the only place that writes model files; the registry is the only
  place that records they are ready, and it does so only with a hash.
- Tokens are optional, never logged, and the `HubClient` `Debug` impl redacts them
  (a derived `Debug` would leak the token on the first log line).
- Network tests are opt-in (`GGUF_HUB_TESTS=1`) and fail loudly otherwise; a
  silently skipped network test is worse than a failing one.

## Verification

- 5 opt-in tests against the real Hub: search, tree listing, a real download that
  verifies and commits, a corrupted-hash rejection that leaves nothing behind, and
  a resumed partial file that still verifies.
- End-to-end: `gguf models pull` downloads `stories260K.gguf`, verifies SHA-256,
  registers it, and `gguf models list` shows the resolved commit and architecture.
