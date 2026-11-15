#!/usr/bin/env bash
# Build the GTK4 shell on Linux (spec 2.1, 4).
#
# Kept as a script rather than a paragraph of instructions because it encodes the
# two things that actually bite: GTK 4 development headers must be installed,
# and the target directory should live on a Linux filesystem when the source tree
# is on a Windows mount (a 9p mount makes a Rust build several times slower).
set -euo pipefail

export PATH="${HOME}/.cargo/bin:${PATH}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${HOME}/target-linux}"

if ! pkg-config --exists gtk4; then
  echo "GTK 4 development headers are missing." >&2
  echo "  Debian/Ubuntu: sudo apt-get install -y libgtk-4-dev build-essential pkg-config" >&2
  echo "  Fedora:        sudo dnf install -y gtk4-devel gcc pkgconf-pkg-config" >&2
  exit 1
fi

echo "GTK 4: $(pkg-config --modversion gtk4)"
echo "target: ${CARGO_TARGET_DIR}"

cd "$(dirname "$0")"
cargo build "$@"
