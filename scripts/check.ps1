#Requires -Version 5.1
# Local equivalent of the CI gate (fmt -> clippy -> tests).
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)

Write-Host '==> cargo fmt' -ForegroundColor Cyan
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host '==> cargo clippy' -ForegroundColor Cyan
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host '==> cargo test' -ForegroundColor Cyan
cargo test --workspace --all-targets
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host 'all checks passed' -ForegroundColor Green
