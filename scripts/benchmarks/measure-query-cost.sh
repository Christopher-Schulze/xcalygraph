#!/usr/bin/env bash
# Report grammar parse, query compile and query match time per language pack of a source tree,
# single-threaded in a release test build; development evidence only.
# Usage: scripts/benchmarks/measure-query-cost.sh [--patterns] ROOT
#   --patterns adds, per pattern, the match time saved by disabling it and its match count.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
patterns=()
if [[ "${1:-}" == "--patterns" ]]; then
  patterns=(XCALYGRAPH_COST_PATTERNS=1)
  shift
fi
(( $# == 1 )) && [[ -d "$1" ]] || { echo "usage: measure-query-cost.sh [--patterns] ROOT" >&2; exit 2; }

cd "${REPO_ROOT}"
env "${patterns[@]}" XCALYGRAPH_COST_ROOT="$1" \
  cargo test --release --locked --quiet --bin xcalygraph query_cost -- --ignored --nocapture |
  grep -v -e '^$' -e '^running ' -e '^test result' -e '^\.$'
