#!/usr/bin/env bash
# Report allocations and time per pipeline stage of a full index build; development evidence
# only, the product binary has no counting allocator.
# Usage: scripts/benchmarks/measure-allocations.sh [--times] ROOT
#   --times builds without the counting allocator, whose contention distorts parallel stages: the
#   milliseconds are comparable, the other columns are zero.
# Output, one tab-separated line per stage and a total: stage, allocations, megabytes allocated,
# live megabytes at the stage end, peak live megabytes during the stage, milliseconds.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
features=alloc-stats
if [[ "${1:-}" == "--times" ]]; then
  features=stage-stats
  shift
fi
(( $# == 1 )) && [[ -d "$1" ]] || { echo "usage: measure-allocations.sh [--times] ROOT" >&2; exit 2; }

target_dir="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
(cd "${REPO_ROOT}" && CARGO_TARGET_DIR="${target_dir}" \
  cargo build --release --locked --quiet --features "${features}")

index_dir="$(mktemp -d "${REPO_ROOT}/.cache/alloc-stats.XXXXXXXX")"
trap 'rm -rf "${index_dir}"' EXIT

XCALYGRAPH_INDEX_DIR="${index_dir}" "${target_dir}/release/xcalygraph" index -C "$1" 2>&1 >/dev/null |
  awk -F '\t' -v OFS='\t' '
    $1 == "alloc-stats" {
      count += $3; bytes += $4; time += $7; if ($6 > peak) peak = $6
      print $2, $3, int($4 / 1048576), int($5 / 1048576), int($6 / 1048576), $7
    }
    END { print "total", count, int(bytes / 1048576), "", int(peak / 1048576), time }'
