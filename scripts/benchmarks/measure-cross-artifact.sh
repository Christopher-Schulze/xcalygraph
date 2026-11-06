#!/usr/bin/env bash
# Share of required roles a single `find` answer shows, or states as absent, at a budget.
# Usage: scripts/benchmarks/measure-cross-artifact.sh BINARY [BUDGET]
# Cases: scripts/benchmarks/questions/cross-artifact.tsv (corpus, question, role, path or absent).
# A role with a path counts when the path stands in the answer; `absent` counts only when the
# answer states `-role`, or names no document for it.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
(( $# >= 1 && $# <= 2 )) && [[ -x "$1" ]] || { echo "usage: measure-cross-artifact.sh BINARY [BUDGET]" >&2; exit 2; }
readonly binary="$1" budget="${2:-600}"
work="$(mktemp -d "${TMPDIR:-/tmp}/xcalygraph-cross.XXXXXXXX")"
trap 'rm -rf -- "${work}"' EXIT
met=0 total=0
while IFS=$'\t' read -r corpus question role path; do
  # shellcheck disable=SC2086  # the question is split into search terms
  answer="$(XCALYGRAPH_INDEX_DIR="${work}" "${binary}" find ${question} -C "${REPO_ROOT}/research/corpora/${corpus}" -b "${budget}" 2>/dev/null || true)"
  verdict=missing
  if [[ "${path}" == absent ]]; then
    grep -qF -- "-${role}" <<<"${answer}" && verdict=met
  else
    grep -qF -- "${path}" <<<"${answer}" && verdict=met
  fi
  total=$((total + 1))
  [[ "${verdict}" == met ]] && met=$((met + 1))
  printf '%s\t%s\t%s\t%s\n' "${verdict}" "${corpus}" "${role}" "${question}"
done < "${REPO_ROOT}/scripts/benchmarks/questions/cross-artifact.tsv"
echo "roles met ${met} of ${total}"
