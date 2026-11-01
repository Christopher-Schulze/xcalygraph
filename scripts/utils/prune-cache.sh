#!/usr/bin/env bash
# Remove aged, explicitly marked completed runs in known cache categories; report total size.
# Usage: scripts/utils/prune-cache.sh [AGE_HOURS]  (default 24; 0 selects all marked runs)
# Unknown entries, candidate symlinks, incomplete runs, snapshots and .keep-protected runs stay.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
readonly REPO_ROOT
readonly CACHE="${REPO_ROOT}/.cache"
readonly AGE_HOURS="${1:-24}"
readonly WARN_KIB=$((3 * 1024 * 1024))
readonly MARKER=".xcalygraph-disposable"

(( $# <= 1 )) || { echo "usage: prune-cache.sh [AGE_HOURS]" >&2; exit 2; }
[[ "${AGE_HOURS}" =~ ^[0-9]{1,8}$ ]] || { echo "usage: prune-cache.sh [AGE_HOURS]" >&2; exit 2; }
age_hours=$((10#${AGE_HOURS}))
(( age_hours <= 10000000 )) || { echo "usage: prune-cache.sh [AGE_HOURS]" >&2; exit 2; }
if [[ -L "${CACHE}" ]]; then
  echo "prune-cache.sh: ${CACHE} is a symlink, refusing to prune" >&2
  exit 1
fi
[[ -e "${CACHE}" ]] || exit 0
[[ -d "${CACHE}" ]] || { echo "prune-cache.sh: ${CACHE} is not a directory" >&2; exit 1; }

removed=0
age_filter=()
(( age_hours > 0 )) && age_filter=(-mmin "+$((age_hours * 60 - 1))")

# Remove only a completed run with the expected marker and no protected descendant.
prune_children() {
  local dir="$1"
  shift
  [[ -d "${dir}" && ! -L "${dir}" ]] || return 0
  [[ ! -e "${dir}/.keep" && ! -L "${dir}/.keep" ]] || return 0
  local entry name pattern protected matched
  while IFS= read -r -d '' entry; do
    name="$(basename "${entry}")"
    [[ -d "${entry}" && ! -L "${entry}" ]] || continue
    matched=0
    for pattern in "$@"; do
      # shellcheck disable=SC2053
      if [[ "${name}" == ${pattern} ]]; then
        matched=1
        break
      fi
    done
    (( matched )) || continue
    [[ -f "${entry}/${MARKER}" && ! -L "${entry}/${MARKER}" ]] || continue
    cmp -s "${entry}/${MARKER}" <(printf 'xcalygraph-cache-v1\n') || continue
    if ! protected="$(find "${entry}" -name .keep -print -quit 2>/dev/null)"; then
      echo "prune-cache.sh: cannot inspect ${entry}; preserving it" >&2
      continue
    fi
    [[ -z "${protected}" ]] || continue
    rm -rf -- "${entry}"
    removed=$((removed + 1))
  done < <(find "${dir}" -mindepth 1 -maxdepth 1 ${age_filter[@]+"${age_filter[@]}"} -print0)
}

if [[ ! -e "${CACHE}/.keep" && ! -L "${CACHE}/.keep" ]]; then
  prune_children "${CACHE}/benchmarks" 'run.*' 'query.*' 'shard-recovery.*' 'index-validation.*'
  prune_children "${CACHE}/depth" 'run.*'
  prune_children "${CACHE}/evaluation" 'run.*' 'sealed.*'
fi

if ! size_kib="$(du -sk "${CACHE}" | awk 'NR == 1 { print $1 }')"; then
  echo "prune-cache.sh: cannot measure cache size" >&2
  exit 1
fi
echo "prune-cache.sh: removed ${removed} entries, .cache is $((size_kib / 1024)) MiB"
if (( size_kib > WARN_KIB )); then
  echo "prune-cache.sh: .cache exceeds 3 GiB; largest entries:" >&2
  du -sk "${CACHE}"/* "${CACHE}"/*/* 2>/dev/null | sort -rn | head -5 \
    | awk '{ printf "  %d MiB %s\n", $1 / 1024, $2 }' >&2
fi
