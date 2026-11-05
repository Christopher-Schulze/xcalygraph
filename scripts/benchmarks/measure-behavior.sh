#!/usr/bin/env bash
# Rank of the operation that answers behavior questions, in `find` output at equal budget.
# Usage: scripts/benchmarks/measure-behavior.sh BINARY [BUDGET]
# Cases: scripts/benchmarks/questions/behavior.tsv (corpus, question, implementing name).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
(( $# >= 1 && $# <= 2 )) && [[ -x "$1" ]] || { echo "usage: measure-behavior.sh BINARY [BUDGET]" >&2; exit 2; }
readonly binary="$1" budget="${2:-600}"
work="$(mktemp -d "${TMPDIR:-/tmp}/xcalygraph-behavior.XXXXXXXX")"
trap 'rm -rf -- "${work}"' EXIT
found=0 total=0
while IFS=$'\t' read -r corpus question name; do
  # shellcheck disable=SC2086  # the question is split into search terms
  if XCALYGRAPH_INDEX_DIR="${work}" "${binary}" find ${question} -C "${REPO_ROOT}/research/corpora/${corpus}" -b "${budget}" 2>/dev/null \
    | grep -qF -- "${name##*.}"; then
    found=$((found + 1))
    verdict=shown
  else
    verdict=missing
  fi
  total=$((total + 1))
  printf '%s\t%s\t%s\t%s\n' "${verdict}" "${corpus}" "${name}" "${question}"
done < "${REPO_ROOT}/scripts/benchmarks/questions/behavior.tsv"
echo "shown ${found} of ${total}"
