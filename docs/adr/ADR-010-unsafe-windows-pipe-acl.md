# ADR-010 — Narrow `unsafe` exception for the Windows pipe DACL

**Status:** Accepted (implementation decision for Task 2)

## Context

The workspace forbids `unsafe` (`[workspace.lints.rust] unsafe_code = "forbid"`).
That rule is worth keeping: nearly all of this product is ordinary safe Rust,
and an `unsafe` block in a scheduler or a parser is a bug factory.

Task 2 needed a local IPC transport that only the owning user can reach. On
Windows the correct mechanism is a named pipe created with a security
descriptor that grants the current user and SYSTEM and nobody else. Tokio's
safe API for creating pipes passes a **null** `SECURITY_ATTRIBUTES`, which
means the pipe inherits the process default DACL. On a typical desktop that
includes the local `Users` group, so every local account could load models and
query the API. The only tokio entry point that accepts a descriptor is
`create_with_security_attributes_raw`, which is `unsafe`.

The Win32 calls needed to build the descriptor (`OpenProcessToken`,
`GetTokenInformation`, `ConvertStringSecurityDescriptorToSecurityDescriptorW`)
are also inherently unsafe.

## Decision

Allow `unsafe` in exactly two modules, both Windows-only and both in
`gguf-engine-security`:

| Module | Why |
| --- | --- |
| `windows_acl` | Resolve the current user's SID and build the `SECURITY_ATTRIBUTES` from SDDL. |
| `windows_pipe` | Call `ServerOptions::create_with_security_attributes_raw` with that descriptor. |

Everything else stays forbidden:

* the crate re-establishes `#![deny(unsafe_code)]` at its root;
* each module carries a scoped `#[allow(unsafe_code)]`;
* every block has a `// SAFETY:` comment naming the invariant it relies on;
* the CI job `deny-unsafe` fails the build if any *other* crate re-enables the
  lint, and this ADR is the only sanctioned reason for doing so.

The alternative — trusting the process default DACL — was rejected: a silently
world-readable control socket is exactly the kind of defect that is invisible in
review and exploitable in practice.

## Consequences

- The safe surface exposed to the rest of the workspace is
  `windows_pipe::create(name) -> PipeInstance` and `pipe_acl_sddl()`; callers
  never see a raw pointer.
- A reviewer must read two small files carefully instead of auditing the whole
  codebase for pointer arithmetic.
- If tokio ever exposes a safe API for security descriptors, both modules should
  be deleted and this ADR marked `Superseded`.
- On Unix there is no exception at all: the socket lives in a `0700` directory,
  which needs no FFI.

## Verification

- `windows_acl` tests assert the generated SDDL grants SYSTEM and the current
  user and no ACE for Everyone/Authenticated Users, and that the OS parser
  accepts it.
- `windows_pipe` tests create a real pipe and connect a client, proving the
  DACL does not lock out its own creator.
- `tests/tests/ipc_e2e.rs` starts the real daemon and talks to it over the real
  pipe, so a broken descriptor surfaces as a failing end-to-end test, not a
  review comment.
