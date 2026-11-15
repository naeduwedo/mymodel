# Contributing

## Before you push

Run the same gate CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

On Windows (PowerShell) the equivalent one-liner is:

```powershell
cargo fmt --all; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace --all-targets
```

## Rules

- `unsafe` is forbidden workspace-wide (`[workspace.lints.rust]`). If a crate
  genuinely needs it, add an ADR first and change the lint there only.
- Do not add a dependency without a reason that fits the spec (§2.2 lists the
  intended set). Prefer the workspace dependency table so versions stay
  aligned; `Cargo.lock` is committed.
- Every new module starts with a doc comment stating what is implemented and
  what is **not**. "Planned" crates expose a `STATUS` constant so the
  distinction is machine-checkable.
- Public items need doc comments — every crate sets `#![warn(missing_docs)]`.
- Tests live next to the code (`#[cfg(test)]`) unless they need a real binary
  or cross-crate wiring; those go in `tests/tests/`.
- Never assert on wall-clock timing or host-specific values that CI cannot
  reproduce; inject or derive them instead.

## Adding a task phase

1. Read the matching section of the specification and the ADRs it cites.
2. Update `docs/architecture/task-status.md` in the same commit that changes the
   status.
3. State honestly which parts are mock-tested vs. hardware-tested (spec §14.2).

## Commit messages

Conventional-ish prefixes: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`,
`chore:`. Reference the spec section or task in the body, e.g.
`Refs: spec §3.3, Task 1`.
