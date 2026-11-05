#!/usr/bin/env bash
# Remove aged, explicitly marked completed runs in known cache categories, compact older evidence
# runs and report total size. Compaction uses the same age and completion requirements.
# Usage: scripts/utils/prune-cache.sh [AGE_HOURS]  (default 24; 0 selects all marked runs)
# Unknown entries, candidate symlinks, incomplete runs, snapshots and .keep-protected runs stay.
# Ad-hoc work directories (task*, edge-guard.*, leftovers of failed test fixtures NAME-PID-N) go
# once nothing in them is younger than the age and no open TASK file names them.
# Fails above 3 GiB without the frozen corpus snapshots: evidence must not grow unnoticed.
# Compaction: in evaluation/ and task192-manifests/, runs beyond the newest KEEP_RUNS, in
# edge-oracle/ beyond the newest one, lose every file except top-level regular files up to 1 MiB (reports, identities,
# summaries, ledgers); a run containing .keep-full is never compacted.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
readonly REPO_ROOT
readonly CACHE="${REPO_ROOT}/.cache"
readonly AGE_HOURS="${1:-24}"
readonly LIMIT_KIB=$((3 * 1024 * 1024))
readonly MARKER=".xcalygraph-disposable"
readonly KEEP_RUNS=2
readonly SMALL_KIB=1024

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
compacted=0
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

# Work directories have no completion marker: age, .keep and open TASK references protect them.
prune_work_directories() {
  local entry protected fresh
  while IFS= read -r -d '' entry; do
    if ! protected="$(find "${entry}" -name .keep -print -quit 2>/dev/null)"; then
      echo "prune-cache.sh: cannot inspect ${entry}; preserving it" >&2
      continue
    fi
    [[ -z "${protected}" ]] || continue
    if (( age_hours > 0 )); then
      fresh="$(find "${entry}" -mmin "-$((age_hours * 60))" -print -quit)" || continue
      [[ -z "${fresh}" ]] || continue
    fi
    if [[ -d "${REPO_ROOT}/docs/tasks" ]] && grep -qsF -- "$(basename "${entry}")" "${REPO_ROOT}"/docs/tasks/*.md; then
      continue
    fi
    rm -rf -- "${entry}"
    removed=$((removed + 1))
  done < <(find "${CACHE}" -mindepth 1 -maxdepth 1 -type d \
    \( -name 'task*' -o -name 'edge-guard.*' -o -name '*-[0-9]*-[0-9]*' \) -print0)
}
[[ -e "${CACHE}/.keep" || -L "${CACHE}/.keep" ]] || prune_work_directories

# Keep reports of completed, aged evidence runs, drop their bulky corpora, indexes and builds.
compact_runs() {
  local dir="$1" keep="$2" entry file aged
  [[ -d "${dir}" && ! -L "${dir}" ]] || return 0
  local runs=()
  while IFS= read -r -d '' entry; do runs+=("${entry}"); done < <(python3 -I -c '
import os, sys
base = sys.argv[1]
names = [name for name in os.listdir(base)
         if name.startswith(("run.", "sealed.")) and os.path.isdir(os.path.join(base, name))
         and not os.path.islink(os.path.join(base, name))]
for name in sorted(names, key=lambda name: os.stat(os.path.join(base, name)).st_mtime, reverse=True):
    sys.stdout.write(os.path.join(base, name) + "\0")
' "${dir}")
  local index
  for (( index = keep; index < ${#runs[@]}; index++ )); do
    entry="${runs[index]}"
    [[ -d "${entry}" && ! -L "${entry}" && ! -e "${entry}/.keep-full" && ! -e "${entry}/.compacted" ]] || continue
    [[ -f "${entry}/${MARKER}" && ! -L "${entry}/${MARKER}" ]] || continue
    cmp -s "${entry}/${MARKER}" <(printf 'xcalygraph-cache-v1\n') || continue
    if (( age_hours > 0 )); then
      aged="$(find "${entry}" -maxdepth 0 ${age_filter[@]+"${age_filter[@]}"} -print)" || return 1
      [[ -n "${aged}" ]] || continue
    fi
    while IFS= read -r -d '' file; do
      if [[ -f "${file}" && ! -L "${file}" && "$(du -sk "${file}" | awk '{ print $1 }')" -le SMALL_KIB ]]; then
        continue
      fi
      rm -rf -- "${file}"
    done < <(find "${entry}" -mindepth 1 -maxdepth 1 -print0)
    : > "${entry}/.compacted"
    compacted=$((compacted + 1))
  done
}
compact_runs "${CACHE}/evaluation" "${KEEP_RUNS}"
compact_runs "${CACHE}/task192-manifests" "${KEEP_RUNS}"
# Compiler indexes run to gigabytes; one whole run is the rescoring input.
compact_runs "${CACHE}/edge-oracle" 1

if ! size_kib="$(du -sk "${CACHE}" | awk 'NR == 1 { print $1 }')"; then
  echo "prune-cache.sh: cannot measure cache size" >&2
  exit 1
fi
echo "prune-cache.sh: removed ${removed} entries, compacted ${compacted} runs, .cache is $((size_kib / 1024)) MiB"
snapshot_kib=0
if [[ -d "${CACHE}/corpus-snapshots" ]]; then
  snapshot_kib="$(du -sk "${CACHE}/corpus-snapshots" | awk 'NR == 1 { print $1 }')"
fi
if (( size_kib - snapshot_kib > LIMIT_KIB )); then
  echo "prune-cache.sh: evidence in .cache exceeds 3 GiB; largest entries:" >&2
  du -sk "${CACHE}"/* "${CACHE}"/*/* 2>/dev/null | sort -rn \
    | awk 'NR <= 5 { printf "  %d MiB %s\n", $1 / 1024, $2 }' >&2
  exit 1
fi
