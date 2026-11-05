#!/usr/bin/env bash
# Check commits before a push: no local-only files, no private names, no absolute home paths,
# no co-author trailers.
# Usage: scripts/audits/check-publication.sh [RANGE]
#   RANGE defaults to the commits not yet on origin/main; HEAD alone when there are none.
#   Private patterns (extended regex, one per line, # comments) live in the gitignored
#   scripts/audits/private-patterns.txt; a missing file fails the check.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
readonly REPO_ROOT
readonly PATTERN_FILE="${REPO_ROOT}/scripts/audits/private-patterns.txt"
readonly LOCAL_ONLY_PATTERN='^(AGENTS\.md|CLAUDE\.md|docs/decisions\.md|docs/tasks\.md|docs/tasks(/|$)|research(/|$)|scripts/benchmarks/questions/private(/|$)|scripts/audits/private-patterns\.txt)'
readonly HOME_PATH_PATTERNS=('/Users/[A-Za-z0-9._-]+' '/home/[A-Za-z0-9._-]+/')

cd "${REPO_ROOT}"

if (( $# > 1 )); then
  echo "check-publication.sh: at most one RANGE argument" >&2
  exit 2
fi
if [[ ! -f "${PATTERN_FILE}" ]]; then
  echo "check-publication.sh: missing ${PATTERN_FILE#"${REPO_ROOT}"/}" >&2
  exit 1
fi

range="${1:-}"
if [[ -z "${range}" ]] && git rev-parse --verify --quiet origin/main >/dev/null; then
  range="origin/main..HEAD"
fi
commits=()
if ! history="$(git rev-list "${range:-HEAD}")"; then
  echo "check-publication.sh: cannot enumerate commits" >&2
  exit 1
fi
if [[ -n "${history}" ]]; then
  while IFS= read -r commit; do
    commits+=("${commit}")
  done <<< "${history}"
fi
if (( ${#commits[@]} == 0 )); then
  commits=("$(git rev-parse HEAD)")
fi

patterns="$(mktemp)"
home_patterns="$(mktemp)"
trap 'rm -f "${patterns}" "${home_patterns}"' EXIT
# Both commands read the pattern file; neither writes it.
# shellcheck disable=SC2094
if ! LC_ALL=C tr -d '\000' < "${PATTERN_FILE}" | cmp -s "${PATTERN_FILE}" -; then
  echo "check-publication.sh: private pattern file is unreadable or contains NUL" >&2
  exit 1
fi
pattern_status=0
grep -aEv '^[[:space:]]*(#|$)' "${PATTERN_FILE}" > "${patterns}" 2>/dev/null || pattern_status=$?
if (( pattern_status > 1 )); then
  echo "check-publication.sh: cannot read private patterns" >&2
  exit 1
fi

scan() {
  local label="$1"
  shift
  scan_status=0
  scan_error="$(
    command_status=0
    "$@" 2>&1 >/dev/null || command_status=$?
    printf '.'
    exit "${command_status}"
  )" || scan_status=$?
  scan_error="${scan_error%.}"
  if (( scan_status > 1 )) || [[ -n "${scan_error}" ]]; then
    if [[ -n "${scan_error}" ]]; then
      echo "check-publication.sh: ${label} reported diagnostics (exit ${scan_status})" >&2
    else
      echo "check-publication.sh: ${label} failed (exit ${scan_status})" >&2
    fi
    return 1
  fi
}

scan_blob() {
  local oid="$1" pattern_file="$2" insensitive="$3"
  local -a arguments=(-a -E -f "${pattern_file}") result
  if [[ "${insensitive}" == 1 ]]; then arguments+=(-i); fi
  if git cat-file blob "${oid}" | grep "${arguments[@]}" >/dev/null; then
    result=("${PIPESTATUS[@]}")
  else
    result=("${PIPESTATUS[@]}")
  fi
  if (( result[0] != 0 || result[1] > 1 )); then return 2; fi
  return "${result[1]}"
}

scan 'private pattern validation' grep -q -i -E -f "${patterns}" /dev/null || exit 1
# Home paths match case-sensitively: `/users/` in `app/api/users/route.ts` is no home path.
printf '%s\n' "${HOME_PATH_PATTERNS[@]}" > "${home_patterns}"

findings=0
for commit in "${commits[@]}"; do
  short="$(git rev-parse --short "${commit}")"
  if ! local_only="$(
    export LC_ALL=C
    git ls-tree -r -t --full-tree --name-only -z "${commit}" | while IFS= read -r -d '' path; do
      if [[ "${path}" =~ ${LOCAL_ONLY_PATTERN} ]]; then printf '%q\n' "${path}"; fi
    done
  )"; then
    echo "${short}: cannot enumerate tracked paths" >&2
    exit 1
  fi
  if [[ -n "${local_only}" ]]; then
    printf '%s\n' "${local_only}"
    echo "${short}: local-only file tracked" >&2
    findings=1
  fi
  # The vendored Unicode license is verbatim third-party text and may contain a common word.
  scan "${short}: private content scan" git grep -q -a -i -E -f "${patterns}" "${commit}" -- ':(top)' \
    ':(top,exclude)vendor/tree-sitter/src/unicode/LICENSE' || exit 1
  private_match="${scan_status}"
  scan "${short}: home path content scan" git grep -q -a -E -f "${home_patterns}" "${commit}" -- ':(top)' || exit 1
  tree_match=0
  if (( private_match == 0 || scan_status == 0 )); then
    tree_match=1
  fi
  if ! symlink_ids="$(git ls-tree -r --full-tree -z "${commit}" | while IFS= read -r -d '' entry; do
    metadata="${entry%%$'\t'*}"
    if [[ "${metadata}" == '120000 blob '* ]]; then printf '%s\n' "${metadata##* }"; fi
  done)"; then
    echo "${short}: cannot enumerate symlink blobs" >&2
    exit 1
  fi
  if [[ -n "${symlink_ids}" ]]; then
    while IFS= read -r oid; do
      scan "${short}: private symlink scan" scan_blob "${oid}" "${patterns}" 1 || exit 1
      private_match="${scan_status}"
      scan "${short}: home path symlink scan" scan_blob "${oid}" "${home_patterns}" 0 || exit 1
      if (( private_match == 0 || scan_status == 0 )); then tree_match=1; fi
    done <<< "${symlink_ids}"
  fi
  if (( tree_match )); then
    echo "${short}: private pattern in tree" >&2
    findings=1
  fi
  if ! message="$(git log -1 --format=%B "${commit}" 2>/dev/null)"; then
    echo "${short}: cannot read commit message" >&2
    exit 1
  fi
  scan "${short}: trailer scan" grep -q -i '^co-authored-by:' <<< "${message}" || exit 1
  if (( scan_status == 0 )); then
    echo "${short}: co-author trailer in commit message" >&2
    findings=1
  fi
  scan "${short}: private message scan" grep -q -i -E -f "${patterns}" <<< "${message}" || exit 1
  private_match="${scan_status}"
  scan "${short}: home path message scan" grep -q -E -f "${home_patterns}" <<< "${message}" || exit 1
  if (( private_match == 0 || scan_status == 0 )); then
    echo "${short}: private pattern in commit message" >&2
    findings=1
  fi
done

if (( findings )); then
  echo "check-publication.sh: findings in ${#commits[@]} commit(s); do not push" >&2
  exit 1
fi
echo "check-publication.sh: ${#commits[@]} commit(s) clean"
