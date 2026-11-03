#!/usr/bin/env bash
# Run the gate: safe cache pruning and Rust checks; --all adds ShellCheck and the script regressions.
# Usage: scripts/tests/test.sh [--all]
# Run --all when scripts/ changed and before a release.
set -euo pipefail

mode=product
case "${1-}" in
  "") ;;
  --all) mode=all ;;
  *) echo "usage: test.sh [--all]" >&2; exit 2 ;;
esac
(( $# <= 1 )) || { echo "usage: test.sh [--all]" >&2; exit 2; }
readonly mode
# macOS paths are case-insensitive; Cargo reports the physical spelling, so use it too.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
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

required_tools=(cargo git python3)
[[ "${mode}" == product ]] || required_tools+=(graphify shellcheck hyperfine jq rsync)
for tool in "${required_tools[@]}"; do
  command -v "${tool}" >/dev/null || { echo "test.sh: ${tool} is required" >&2; exit 1; }
done
if [[ "${mode}" == all ]]; then
  readonly expected_graphify_version="graphify 0.9.97"
  graphify_version="$(graphify --version)" || { echo "test.sh: could not read Graphify version" >&2; exit 1; }
  [[ "${graphify_version}" == "${expected_graphify_version}" ]] || {
    echo "test.sh: expected ${expected_graphify_version}, got ${graphify_version}" >&2
    exit 1
  }
  find "${REPO_ROOT}/scripts" -type f -name '*.sh' -print0 | xargs -0 shellcheck
fi

CARGO_TARGET_DIR="$(bash "${REPO_ROOT}/scripts/utils/verification-target.sh")"
CARGO_BUILD_BUILD_DIR="${CARGO_TARGET_DIR}"
export CARGO_TARGET_DIR CARGO_BUILD_BUILD_DIR
bash "${REPO_ROOT}/scripts/utils/verification-source.sh" \
  bash "${REPO_ROOT}/scripts/tests/test-execution.sh" "${work}" "${mode}"
