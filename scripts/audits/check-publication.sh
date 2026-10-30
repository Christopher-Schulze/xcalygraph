#!/usr/bin/env bash
# Check commits before a push: no local-only files, no private names, no absolute home paths.
# Usage: scripts/audits/check-publication.sh [RANGE]
#   RANGE defaults to the commits not yet on origin/main; HEAD alone when there are none.
#   Private patterns (extended regex, one per line, # comments) live in the gitignored
#   scripts/audits/private-patterns.txt; a missing file fails the check.
#   LICENSE-APACHE is excluded from the pattern search because the canonical license text uses
#   common words; instead it must match the canonical Apache-2.0 text byte for byte (SHA-256).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly PATTERN_FILE="${REPO_ROOT}/scripts/audits/private-patterns.txt"
readonly LOCAL_ONLY_PATTERN='^(AGENTS\.md|CLAUDE\.md|docs/decisions\.md|docs/tasks\.md|docs/tasks/|research/|scripts/benchmarks/questions/private/|scripts/audits/private-patterns\.txt)'
readonly HOME_PATH_PATTERNS=('/Users/[A-Za-z0-9._-]+' '/home/[A-Za-z0-9._-]+/')
readonly APACHE_LICENSE_SHA256='cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30'

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
trap 'rm -f "${patterns}"' EXIT
grep -Ev '^[[:space:]]*(#|$)' "${PATTERN_FILE}" > "${patterns}" || true
printf '%s\n' "${HOME_PATH_PATTERNS[@]}" >> "${patterns}"

findings=0
for commit in "${commits[@]}"; do
  short="$(git rev-parse --short "${commit}")"
  if git ls-tree -r --name-only "${commit}" | grep -E "${LOCAL_ONLY_PATTERN}"; then
    echo "${short}: local-only file tracked" >&2
    findings=1
  fi
  if git grep -n -I -i -E -f "${patterns}" "${commit}" -- . ':(exclude)LICENSE-APACHE'; then
    echo "${short}: private pattern in tree" >&2
    findings=1
  fi
  if git cat-file -e "${commit}:LICENSE-APACHE" 2>/dev/null \
    && [[ "$(git show "${commit}:LICENSE-APACHE" | shasum -a 256 | cut -d' ' -f1)" != "${APACHE_LICENSE_SHA256}" ]]; then
    echo "${short}: LICENSE-APACHE differs from the canonical Apache-2.0 text" >&2
    findings=1
  fi
  if git log -1 --format=%B "${commit}" | grep -n -i -E -f "${patterns}"; then
    echo "${short}: private pattern in commit message" >&2
    findings=1
  fi
done

if (( findings )); then
  echo "check-publication.sh: findings in ${#commits[@]} commit(s); do not push" >&2
  exit 1
fi
echo "check-publication.sh: ${#commits[@]} commit(s) clean"
