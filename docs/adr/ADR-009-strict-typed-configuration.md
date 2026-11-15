# ADR-009 — Strict, typed configuration with path containment

**Status:** Accepted (implementation decision for Task 1)

## Context

Spec §11 requires a schema with defaults, range validation and UI help for
every option; §9.3 requires that file paths be normalised and confined to the
configured directories. Hand-rolled `serde_json::Value` configuration gets
these wrong: typos are silently ignored, and a `../` in `paths.models` can
redirect a 40 GB download into the user's home directory.

## Decision

- Configuration is a typed Rust structure with `#[serde(deny_unknown_fields)]`
  — an unknown key is an error, because a silently ignored setting is a lie.
- Defaults are the specification's example values, asserted by a test.
- Validation is split into `check()` (all findings, for the UI) and
  `validate()` (fail on errors), so the Settings page can show every problem
  at once.
- Every configured directory is resolved against one data root and rejected
  with `PathEscape` if it resolves outside it.
- Security defaults are enforced in the schema: a non-loopback API bind
  requires token authentication, or the configuration is invalid.

## Consequences

- Upgrading the option set is a code change with tests, not a config-file
  migration guess.
- Users who want data elsewhere get an explicit error telling them the path
  must stay inside the data root, instead of a silent write to a random place.
- Task 2's `settings` table can store per-key overrides because the key set is
  known statically.
