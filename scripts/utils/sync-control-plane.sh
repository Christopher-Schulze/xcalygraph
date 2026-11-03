#!/usr/bin/env bash
# Commit and push the local-only control plane (agent rules, TASK files, decisions, private
# benchmark inputs) to its private repository, kept in a separate Git directory.
# Usage: scripts/utils/sync-control-plane.sh [MESSAGE]
#   First use: git --git-dir=.control.git init, then set its `origin` remote to a private repository.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
readonly CONTROL="${REPO_ROOT}/.control.git"
readonly FILES=(AGENTS.md CLAUDE.md docs/tasks.md docs/tasks docs/decisions.md
  scripts/audits/private-patterns.txt scripts/benchmarks/questions/private)
(( $# <= 1 )) || { echo "usage: sync-control-plane.sh [MESSAGE]" >&2; exit 2; }
[[ -d "${CONTROL}" && ! -L "${CONTROL}" ]] || { echo "sync-control-plane.sh: ${CONTROL} missing; see usage" >&2; exit 1; }
control() { git --git-dir="${CONTROL}" --work-tree="${REPO_ROOT}" "$@"; }
present=()
for file in "${FILES[@]}"; do [[ -e "${REPO_ROOT}/${file}" ]] && present+=("${file}"); done
control add --force --all -- "${present[@]}"
if control diff --cached --quiet; then
  echo "sync-control-plane.sh: no changes"
else
  control commit --quiet -m "${1:-Sync control plane}"
  echo "sync-control-plane.sh: committed $(control rev-parse --short HEAD)"
fi
control push --quiet origin HEAD:main
