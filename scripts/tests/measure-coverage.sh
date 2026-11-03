#!/usr/bin/env bash
# Report line coverage of src/ without test-only code and development tools.
# Usage: scripts/tests/measure-coverage.sh [--missing]
#   --missing also lists uncovered lines per file. Requires cargo-llvm-cov and the llvm-tools component.
set -euo pipefail
(( $# == 0 )) || [[ "$*" == --missing ]] || { echo "usage: measure-coverage.sh [--missing]" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
cd "${REPO_ROOT}"
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
flags=(--summary-only)
[[ "$*" != --missing ]] || flags+=(--show-missing-lines)
exec cargo llvm-cov --locked "${flags[@]}" \
  --ignore-filename-regex '(tests\.rs|/tests/|golden\.rs|testdir\.rs|query/benchmark\.rs)'
