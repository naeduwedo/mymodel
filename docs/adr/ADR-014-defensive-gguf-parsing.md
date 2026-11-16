> **English** | [中文](ADR-014-defensive-gguf-parsing.zh.md)

# ADR-014 — GGUF parsing: bounded prefix read, no mmap

**Status:** Accepted (implementation decision for Task 5)

## Context

Spec §7.1 requires a read-only GGUF parser that defends against malformed files:
integer overflow, out-of-range offsets, absurd lengths, duplicate keys and
invalid strings. The obvious implementation — `mmap` the file and index into it —
has a subtle failure mode: a file that shrinks or is truncated after mapping
causes a `SIGBUS` (not a catchable error) when the parser touches a page beyond
the current file size. That kills the process, which is exactly what a parser for
untrusted input must not do.

## Decision

Read only the front of the file:

- the header, metadata and tensor table live before the tensor payloads, so a
  bounded prefix read (`HEADER_READ_LIMIT` = 256 MiB) contains everything the
  parser needs;
- a 100 GB model therefore costs a few megabytes of memory to inspect, and there
  is no `mmap`, no `unsafe`, and no `SIGBUS`;
- every read goes through a cursor that cannot leave the buffer; every length
  from the file is checked against `Limits` *before* it sizes anything;
- tensor bounds are validated against the real file length (passed in with the
  prefix), so a tensor pointing past the end is rejected even though its bytes
  were never read.

## Consequences

- **No `unsafe`** anywhere in the parser; the workspace-wide ban holds.
- Malformed input produces an error with a byte offset, never a panic.
- An unrecognised tensor type keeps its numeric id and is reported as `unknown(N)`,
  so a compatibility report cannot mistake "type 27" for a known quantisation.
- Big-endian files (the historical `-be` quants) are named as such instead of
  reporting an absurd version number — discovered against a real file.

## Verification

- 35 unit tests: 20 synthetic malformed cases (bad magic, truncated header, 2^40
  metadata count, duplicate keys, out-of-range tensor, overflow, bad alignment,
  unknown types, deep array nesting) plus a real-file test that must run (it
  fails loudly when no sample is present).
- A real pull + inspect over the network parses the downloaded file and reports
  its real architecture, not a guess.
