#!/usr/bin/env sh
# Local equivalent of the CI gate (fmt -> clippy -> tests).
set -eu
cd "$(dirname "$0")/.."

echo "==> cargo fmt"
cargo fmt --all -- --check

echo "==> cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test"
cargo test --workspace --all-targets

echo "all checks passed"
