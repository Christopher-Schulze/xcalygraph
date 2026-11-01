#!/usr/bin/env bash
# Keep the development cache (.cache/) small: remove ephemeral test, profiling and benchmark
# artifacts older than AGE_HOURS and report the remaining size.
# Usage: scripts/utils/prune-cache.sh [AGE_HOURS]  (default 24; 0 removes every ephemeral entry)
#   Durable, never pruned: corpus-snapshots/, evaluation/exposure/, evaluation/upstream-*,
#   evaluation/index-*, generalization/, agent-benchmark/, index/ and any directory that
#   contains a .keep file (for example a sealed evaluation run used as --reuse-graphify baseline).
#   Pruned by age: every other top-level entry, children of benchmarks/ and depth/, and
#   evaluation/run.* and evaluation/sealed.* runs.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly CACHE="${REPO_ROOT}/.cache"
readonly AGE_HOURS="${1:-24}"
readonly WARN_KIB=$((3 * 1024 * 1024))
readonly DURABLE_TOP="corpus-snapshots evaluation generalization agent-benchmark index"

[[ "${AGE_HOURS}" =~ ^[0-9]+$ ]] || { echo "usage: prune-cache.sh [AGE_HOURS]" >&2; exit 2; }
[[ -d "${CACHE}" ]] || exit 0
if [[ -L "${CACHE}" ]]; then
  echo "prune-cache.sh: ${CACHE} is a symlink, refusing to prune" >&2
  exit 1
fi

removed=0
age_filter=()
(( AGE_HOURS > 0 )) && age_filter=(-mmin "+$((AGE_HOURS * 60 - 1))")

# Removes each direct child of DIR older than AGE_HOURS whose name matches one of the globs.
prune_children() {
  local dir="$1"
  shift
  [[ -d "${dir}" && ! -L "${dir}" ]] || return 0
  local entry name pattern
  while IFS= read -r -d '' entry; do
    name="$(basename "${entry}")"
    [[ -e "${entry}/.keep" ]] && continue
    for pattern in "$@"; do
      # shellcheck disable=SC2053
      if [[ "${name}" == ${pattern} ]]; then
        rm -rf -- "${entry}"
        removed=$((removed + 1))
        break
      fi
    done
  done < <(find "${dir}" -mindepth 1 -maxdepth 1 ${age_filter[@]+"${age_filter[@]}"} -print0)
}

prune_children "${CACHE}/benchmarks" '*'
prune_children "${CACHE}/depth" '*'
prune_children "${CACHE}/evaluation" 'run.*' 'sealed.*'

while IFS= read -r -d '' entry; do
  name="$(basename "${entry}")"
  [[ " ${DURABLE_TOP} " == *" ${name} "* ]] && continue
  [[ -e "${entry}/.keep" ]] && continue
  [[ "${name}" == benchmarks || "${name}" == depth ]] && [[ -n "$(ls -A "${entry}")" ]] && continue
  rm -rf -- "${entry}"
  removed=$((removed + 1))
done < <(find "${CACHE}" -mindepth 1 -maxdepth 1 ${age_filter[@]+"${age_filter[@]}"} -print0)

size_kib="$(du -sk "${CACHE}" | awk '{ print $1 }')"
echo "prune-cache.sh: removed ${removed} entries, .cache is $((size_kib / 1024)) MiB"
if (( size_kib > WARN_KIB )); then
  echo "prune-cache.sh: .cache exceeds 3 GiB; largest entries:" >&2
  du -sk "${CACHE}"/* "${CACHE}"/*/* 2>/dev/null | sort -rn | head -5 \
    | awk '{ printf "  %d MiB %s\n", $1 / 1024, $2 }' >&2
fi
