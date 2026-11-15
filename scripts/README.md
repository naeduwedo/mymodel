# scripts/

Developer helper scripts. Keep them small and cross-platform-aware; CI calls
`cargo` directly so these are convenience only.

- `check.ps1` — Windows: fmt + clippy + tests, the same gate as CI.
- `check.sh` — Linux/macOS equivalent.
