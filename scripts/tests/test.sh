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
cargo fmt --all --check
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
cargo clippy --all-targets --locked -- -D warnings
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
cargo test --locked
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
cargo build --locked --bin xcalygraph --message-format=json > "${work}/build.jsonl"
binary="$(python3 -I - "${work}/build.jsonl" <<'PYTHON'
import json
import sys

artifacts = [json.loads(line) for line in open(sys.argv[1])]
executables = [item["executable"] for item in artifacts
               if item.get("reason") == "compiler-artifact"
               and item["target"]["name"] == "xcalygraph"
               and "bin" in item["target"]["kind"] and item.get("executable")]
if len(executables) != 1:
    raise SystemExit("test.sh: Cargo did not report exactly one xcalygraph binary")
print(executables[0])
PYTHON
)"
bash "${REPO_ROOT}/scripts/tests/test-prune-cache.sh"
bash "${REPO_ROOT}/scripts/tests/test-agent-benchmark-safety.sh"
bash "${REPO_ROOT}/scripts/tests/test-benchmark-inputs.sh"
bash "${REPO_ROOT}/scripts/tests/test-build-space.sh"
bash "${REPO_ROOT}/scripts/tests/test-hyperfine-args.sh"
bash "${REPO_ROOT}/scripts/tests/test-verification-target.sh"
bash "${REPO_ROOT}/scripts/tests/test-cache-producers.sh" "${binary}"
bash "${REPO_ROOT}/scripts/tests/test-corpus-snapshots.sh" "${binary}"
bash "${REPO_ROOT}/scripts/tests/test-evaluation-labels.sh" "${binary}"
