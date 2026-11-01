#!/usr/bin/env bash
# Run the full gate: safe cache pruning, shell and Rust checks, and real-tool regressions.
# Usage: scripts/tests/test.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT

if [[ -n "${XCALYGRAPH_BLESS+x}" ]]; then
  echo "test.sh: XCALYGRAPH_BLESS is set; refusing to run the gate" >&2
  exit 2
fi
readonly TOOLCHAIN_HOME="${RUSTUP_HOME:-${HOME}/.rustup}"
readonly PACKAGE_HOME="${CARGO_HOME:-${HOME}/.cargo}"
for variable in ${!GIT_@} ${!XCALYGRAPH_@}; do unset "${variable}"; done
unset BASH_ENV ENV CDPATH TAR_OPTIONS

cd "${REPO_ROOT}"
bash "${REPO_ROOT}/scripts/utils/prune-cache.sh"
mkdir -p "${REPO_ROOT}/.cache"
work="$(mktemp -d "${REPO_ROOT}/.cache/test-gate.XXXXXXXX")"
readonly work
trap 'rm -rf -- "${work}"' EXIT
mkdir "${work}/home" "${work}/temporary"
export HOME="${work}/home" TMPDIR="${work}/temporary" XDG_CONFIG_HOME="${work}/home/config"
export RUSTUP_HOME="${TOOLCHAIN_HOME}" CARGO_HOME="${PACKAGE_HOME}"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 RUSTFLAGS="-Dwarnings"

for tool in cargo git python3 graphify shellcheck hyperfine; do
  command -v "${tool}" >/dev/null || { echo "test.sh: ${tool} is required" >&2; exit 1; }
done
find "${REPO_ROOT}/scripts" -type f -name '*.sh' -print0 | xargs -0 shellcheck

CARGO_TARGET_DIR="$(bash "${REPO_ROOT}/scripts/utils/verification-target.sh")"
CARGO_BUILD_BUILD_DIR="${CARGO_TARGET_DIR}"
export CARGO_TARGET_DIR CARGO_BUILD_BUILD_DIR
bash "${REPO_ROOT}/scripts/utils/verification-source.sh" \
  bash "${REPO_ROOT}/scripts/tests/test-execution.sh" "${work}"
