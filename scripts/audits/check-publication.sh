#!/usr/bin/env bash
# Check commits before a push: no local-only files, no private names, no absolute home paths,
# no co-author trailers.
# Usage: scripts/audits/check-publication.sh [RANGE]
#   RANGE defaults to the commits not yet on origin/main; HEAD alone when there are none.
#   Private patterns (extended regex, one per line, # comments) live in the gitignored
#   scripts/audits/private-patterns.txt; a missing file fails the check.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly PATTERN_FILE="${REPO_ROOT}/scripts/audits/private-patterns.txt"
readonly LOCAL_ONLY_PATTERN='^(AGENTS\.md|CLAUDE\.md|docs/decisions\.md|docs/tasks\.md|docs/tasks/|research/|scripts/benchmarks/questions/private/|scripts/audits/private-patterns\.txt)'
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
while IFS= read -r commit; do
  commits+=("${commit}")
done < <(git rev-list "${range:-HEAD}")
if (( ${#commits[@]} == 0 )); then
  commits=("$(git rev-parse HEAD)")
fi

patterns="$(mktemp)"
home_patterns="$(mktemp)"
trap 'rm -f "${patterns}" "${home_patterns}"' EXIT
grep -Ev '^[[:space:]]*(#|$)' "${PATTERN_FILE}" > "${patterns}" || true
# Home paths match case-sensitively: `/users/` in `app/api/users/route.ts` is no home path.
printf '%s\n' "${HOME_PATH_PATTERNS[@]}" > "${home_patterns}"

findings=0
for commit in "${commits[@]}"; do
  short="$(git rev-parse --short "${commit}")"
  if git ls-tree -r --name-only "${commit}" | grep -E "${LOCAL_ONLY_PATTERN}"; then
    echo "${short}: local-only file tracked" >&2
    findings=1
  fi
  if git grep -n -I -i -E -f "${patterns}" "${commit}" -- . \
    || git grep -n -I -E -f "${home_patterns}" "${commit}" -- .; then
    echo "${short}: private pattern in tree" >&2
    findings=1
  fi
  if git log -1 --format=%B "${commit}" | grep -n -i '^co-authored-by:'; then
    echo "${short}: co-author trailer in commit message" >&2
    findings=1
  fi
  message="$(git log -1 --format=%B "${commit}")"
  if grep -n -i -E -f "${patterns}" <<<"${message}" || grep -n -E -f "${home_patterns}" <<<"${message}"; then
    echo "${short}: private pattern in commit message" >&2
    findings=1
  fi
done

if (( findings )); then
  echo "check-publication.sh: findings in ${#commits[@]} commit(s); do not push" >&2
  exit 1
fi
echo "check-publication.sh: ${#commits[@]} commit(s) clean"
