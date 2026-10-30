#!/usr/bin/env bash
# Clone or refresh the Graphify reference clone used for clean-room analysis.
# Usage: scripts/utils/sync-upstream.sh [COMMIT]  (default: upstream default branch)
set -euo pipefail

readonly UPSTREAM_URL="https://github.com/Graphify-Labs/graphify"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly TARGET_DIR="${REPO_ROOT}/research/upstream/graphify"

if [[ -d "${TARGET_DIR}/.git" ]]; then
  git -C "${TARGET_DIR}" fetch --quiet origin
else
  mkdir -p "$(dirname "${TARGET_DIR}")"
  git clone --quiet "${UPSTREAM_URL}" "${TARGET_DIR}"
fi

git -C "${TARGET_DIR}" checkout --quiet --detach "${1:-origin/HEAD}"
git -C "${TARGET_DIR}" log -1 --format='%H %cs %s'
