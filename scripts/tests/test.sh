#!/usr/bin/env bash
# Run the full gate: prune stale development cache, then format check, lint and tests; any
# finding fails the run.
# Usage: scripts/tests/test.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly MIN_FREE_KIB=$((2 * 1024 * 1024))

free_kib() {
  df -Pk "${REPO_ROOT}" | awk 'NR == 2 { print $4 }'
}

cd "${REPO_ROOT}"
bash "${REPO_ROOT}/scripts/utils/prune-cache.sh"

if (( $(free_kib) < MIN_FREE_KIB )); then
  echo "test.sh: less than 2 GiB free, running cargo clean" >&2
  cargo clean
  if (( $(free_kib) < MIN_FREE_KIB )); then
    echo "test.sh: build aborted, less than 2 GiB free after cargo clean; free disk space manually" >&2
    exit 1
  fi
fi

export RUSTFLAGS="-Dwarnings"
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
