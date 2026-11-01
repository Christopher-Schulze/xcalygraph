#!/usr/bin/env bash
# Run a benchmark under this checkout's owned Cargo target and source lock.
# Usage: scripts/utils/guard-benchmark.sh COMMAND [ARG...]
set -euo pipefail
(( $# > 0 )) || { echo "usage: guard-benchmark.sh COMMAND [ARG...]" >&2; exit 2; }
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
cd "${REPO_ROOT}"
CARGO_TARGET_DIR="$(bash "${REPO_ROOT}/scripts/utils/verification-target.sh")"
CARGO_BUILD_BUILD_DIR="${CARGO_TARGET_DIR}"
export CARGO_TARGET_DIR CARGO_BUILD_BUILD_DIR
exec bash "${REPO_ROOT}/scripts/utils/verification-source.sh" --export-lock "$@"
