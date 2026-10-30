#!/usr/bin/env bash
# Fetch the pinned public reference corpora (docs/spec.md section 12) into research/corpora/.
# Usage: scripts/utils/sync-corpora.sh [NAME...]
#   Reads scripts/benchmarks/corpora.tsv (name, url, commit, languages) and checks out each
#   pinned commit with a depth-1 fetch; existing corpora at the pinned commit are left as is.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly LIST="${REPO_ROOT}/scripts/benchmarks/corpora.tsv"
readonly TARGET="${REPO_ROOT}/research/corpora"

wanted=("$@")
selected() {
  (( ${#wanted[@]} == 0 )) && return 0
  local name
  for name in "${wanted[@]}"; do [[ "${name}" == "$1" ]] && return 0; done
  return 1
}

mkdir -p "${TARGET}"
while IFS=$'\t' read -r name url commit _; do
  [[ "${name}" == "name" || -z "${name}" ]] && continue
  selected "${name}" || continue
  dir="${TARGET}/${name}"
  if [[ -d "${dir}/.git" ]] && [[ "$(git -C "${dir}" rev-parse HEAD)" == "${commit}" ]]; then
    echo "${name}: at ${commit}"
    continue
  fi
  mkdir -p "${dir}"
  git -C "${dir}" init -q
  git -C "${dir}" fetch -q --depth 1 "${url}" "${commit}"
  git -C "${dir}" checkout -q --detach FETCH_HEAD
  echo "${name}: checked out ${commit}"
done < "${LIST}"
