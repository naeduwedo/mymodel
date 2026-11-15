#!/usr/bin/env bash
# Build the Linux packages (spec 13 Phase 6, 17 Task 9).
#
# Requires: cargo, nfpm (https://nfpm.goreleaser.com), dpkg and/or rpm tools.
#
# Status: this is the script that turns packaging/linux from "recipe only" into
# "built". Until it has been run somewhere, the README says so.
set -euo pipefail

export PATH="${HOME}/.cargo/bin:${HOME}/go/bin:${PATH}"

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
dist="${repo}/dist"

if ! command -v nfpm >/dev/null 2>&1; then
  echo "nfpm is not installed." >&2
  echo "  go install github.com/goreleaser/nfpm/v2/cmd/nfpm@latest" >&2
  exit 1
fi

# One version source: the workspace Cargo.toml.
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "${repo}/Cargo.toml" | head -1)"
if [[ -z "${version}" ]]; then
  echo "could not read the workspace version from Cargo.toml" >&2
  exit 1
fi
echo "version: ${version}"

echo "== building release binaries"
( cd "${repo}" && cargo build --release \
    -p gguf-engine-daemon \
    -p gguf-engine-cli \
    -p gguf-engine-runtime-worker )

mkdir -p "${dist}"

# The version is substituted here rather than through nfpm's {{ .Env.VERSION }}
# templating: that expands the metadata field but leaves the same text in the
# generated file name, which produced artefacts literally named
# "gguf-ai-engine_{{ .Env.VERSION }}_amd64.deb". Writing the resolved config
# makes the result obvious and the file name correct.
generated="${dist}/nfpm.generated.yaml"
sed "s|{{ .Env.VERSION }}|${version}|g" "${here}/nfpm.yaml" > "${generated}"
if grep -q '{{' "${generated}"; then
  echo "the generated nfpm config still contains a template: ${generated}" >&2
  exit 1
fi

# The GTK4 shell is part of the package: the desktop entry points at it.
# It is built by its own standalone package, into a path the nfpm config names.
echo "== building the GTK4 shell"
( cd "${repo}/platforms/linux" && CARGO_TARGET_DIR="${repo}/platforms/linux/target-shell" cargo build --release )
if [[ ! -x "${repo}/platforms/linux/target-shell/release/gguf-linux-shell" ]]; then
  echo "the GTK4 shell did not build; the desktop entry would point at nothing" >&2
  exit 1
fi

echo "== building .deb"
( cd "${repo}" && nfpm package --config "${generated}" --packager deb --target "${dist}/" )

echo "== building .rpm"
( cd "${repo}" && nfpm package --config "${generated}" --packager rpm --target "${dist}/" )

echo "== built"
ls -l "${dist}"

echo "== verifying the .deb file list"
deb="$(ls "${dist}"/*.deb | head -1)"
dpkg -c "${deb}" > "${dist}/deb-contents.txt"
for expected in \
  ./usr/lib/gguf-ai-engine/gguf-ai-engine-ui \
  ./usr/lib/gguf-ai-engine/gguf-core \
  ./usr/lib/gguf-ai-engine/gguf \
  ./usr/lib/gguf-ai-engine/gguf-worker \
  ./usr/bin/gguf \
  ./usr/share/applications/gguf-ai-engine.desktop
do
  if ! grep -qF "${expected}" "${dist}/deb-contents.txt"; then
    echo "missing from the package: ${expected}" >&2
    cat "${dist}/deb-contents.txt" >&2
    exit 1
  fi
done
echo "   all expected files present"

echo "== PASS"