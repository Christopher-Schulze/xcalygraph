#!/usr/bin/env bash
# Fail when a full index build retires over 5 % more instructions than the blessed baseline; the
# gate runs this so a pipeline regression cannot pass unnoticed. The count ignores machine load.
# Usage: scripts/benchmarks/check-instructions.sh BINARY
#        scripts/benchmarks/check-instructions.sh --bless BINARY ROOT...
# The baseline is local and machine-bound (.cache/instruction-baseline.tsv: root, instructions).
# Without a baseline, or where /usr/bin/time reports no instructions, the check passes silently.
# A deliberate cost is blessed again after its reason is recorded in the TASK Notes.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
readonly BASELINE="${XCALYGRAPH_INSTRUCTION_BASELINE:-${REPO_ROOT}/.cache/instruction-baseline.tsv}"
readonly LIMIT_PERCENT=5

bless=0
if [[ "${1:-}" == "--bless" ]]; then
  bless=1
  shift
fi
(( $# >= 1 + bless )) && [[ -x "$1" ]] || {
  echo "usage: check-instructions.sh [--bless] BINARY [ROOT...]" >&2
  exit 2
}
readonly binary="$1"
shift

work="$(mktemp -d "${TMPDIR:-/tmp}/xcalygraph-instructions.XXXXXXXX")"
readonly work
trap 'rm -rf -- "${work}"' EXIT

# Retired instructions of one fresh build, empty when the platform does not report them.
measure() {
  rm -rf -- "${work}/index"
  XCALYGRAPH_INDEX_DIR="${work}/index" /usr/bin/time -l "${binary}" index -C "$1" 2>&1 >/dev/null |
    awk '/instructions retired/ { print $1 }'
}

if (( bless )); then
  : > "${work}/baseline"
  for root in "$@"; do
    # The lower of two runs: the first one pays for cold pages.
    first="$(measure "${root}")"
    second="$(measure "${root}")"
    printf '%s\t%s\n' "${root}" "$(( first < second ? first : second ))" >> "${work}/baseline"
  done
  mv -- "${work}/baseline" "${BASELINE}"
  exit 0
fi

[[ -f "${BASELINE}" ]] || exit 0
while IFS=$'\t' read -r root blessed; do
  [[ -d "${root}" ]] || continue
  count="$(measure "${root}")"
  [[ -n "${count}" ]] || exit 0
  if (( count * 100 > blessed * (100 + LIMIT_PERCENT) )); then
    # One retry: a single run can pay for cold pages.
    count="$(measure "${root}")"
  fi
  if (( count * 100 > blessed * (100 + LIMIT_PERCENT) )); then
    echo "check-instructions.sh: $(( (count - blessed) * 100 / blessed )) % more instructions than the baseline on $(basename "${root}"): ${blessed} -> ${count}" >&2
    exit 1
  fi
done < "${BASELINE}"
