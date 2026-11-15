# migrations/

Versioned SQLite migrations (Task 2). Forward-only, numbered
`NNNN_description.sql`, applied by `sqlx::migrate!`.

Rules from spec §10: capacities are integer bytes (never floating-point GB),
timestamps are UTC, and every migration ships with an upgrade test and a
backup step.
