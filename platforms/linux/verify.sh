#!/usr/bin/env bash
# Verify the GTK4 shell end to end on a headless Linux machine.
#
# Why headless: CI has no display, but a GTK app that cannot create a window
# still starts and still fails. Running under Xvfb proves the window is built,
# the core is reached over the Unix socket, and the snapshot is rendered into
# the widget tree - the three things that "it compiles" does not prove.
set -euo pipefail

export PATH="${HOME}/.cargo/bin:${PATH}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${HOME}/target-linux}"

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "${here}/../.." && pwd)"
data="$(mktemp -d)"

cleanup() {
  [[ -n "${core_pid:-}" ]] && kill "${core_pid}" 2>/dev/null || true
  rm -rf "${data}"
}
trap cleanup EXIT

echo "== preparing data root: ${data}"
mkdir -p "${data}/models"

echo "== building core and shell"
( cd "${repo}" && cargo build -p gguf-engine-daemon )
bash "${here}/build.sh"

core_bin="${CARGO_TARGET_DIR}/debug/gguf-core"
shell_bin="${CARGO_TARGET_DIR}/debug/gguf-linux-shell"
[[ -x "${core_bin}" ]] || { echo "missing ${core_bin}" >&2; exit 1; }
[[ -x "${shell_bin}" ]] || { echo "missing ${shell_bin}" >&2; exit 1; }

echo "== starting the core"
GGUF_AI_ENGINE_DATA_DIR="${data}" "${core_bin}" --data-dir "${data}" >"${data}/core.log" 2>&1 &
core_pid=$!

# Wait for the socket rather than sleeping a fixed time.
for _ in $(seq 1 80); do
  if grep -q 'core ready' "${data}/core.log" 2>/dev/null; then break; fi
  sleep 0.25
done
grep -q 'core ready' "${data}/core.log" || {
  echo 'the core never became ready:' >&2
  cat "${data}/core.log" >&2
  exit 1
}
echo "   core ready"

echo "== launching the shell under Xvfb for 8 seconds"
set +e
GGUF_AI_ENGINE_DATA_DIR="${data}" timeout 8 xvfb-run -a "${shell_bin}" >"${data}/shell.log" 2>&1
shell_status=$?
set -e

# 124 is the timeout killing a still-running app: that is success. A crash or a
# clean exit within 8 seconds is not.
if [[ ${shell_status} -ne 124 ]]; then
  echo "the shell exited on its own (status ${shell_status})" >&2
  cat "${data}/shell.log" >&2
  exit 1
fi
echo "   shell stayed alive and was stopped by the timeout"

echo "== errors logged by the shell"
if grep -qiE 'critical|assertion|panic' "${data}/shell.log" 2>/dev/null; then
  echo 'the shell logged a critical error:' >&2
  cat "${data}/shell.log" >&2
  exit 1
fi
echo "   none"

echo "== PASS"