#!/usr/bin/env bash
# Check the root and Cargo target filesystems before a build.
# Usage: scripts/utils/check-build-space.sh [MIN_BUILD_KIB]
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly MIN_FREE_KIB=$((2 * 1024 * 1024))
readonly FRESH_BUILD_KIB=$((4 * 1024 * 1024))
estimate="${1:-${FRESH_BUILD_KIB}}"
if (( $# > 1 )) || [[ ! "${estimate}" =~ ^[0-9]{1,12}$ ]] || (( 10#${estimate} == 0 )); then
  echo "usage: check-build-space.sh [MIN_BUILD_KIB]" >&2
  exit 2
fi
estimate=$((10#${estimate}))
(( estimate >= FRESH_BUILD_KIB )) || estimate="${FRESH_BUILD_KIB}"

available_kib() {
  local value
  value="$(df -Pk "$1" | awk 'NR == 2 { print $4 }')"
  [[ "${value}" =~ ^[0-9]+$ ]] || { echo "check-build-space.sh: cannot measure free space: $1" >&2; return 1; }
  printf '%s\n' "${value}"
}

target_parent() {
  local parent
  target_probe="$1"
  while [[ ! -e "${target_probe}" ]]; do
    parent="${target_probe%/*}"
    [[ -n "${parent}" ]] || parent=/
    [[ "${parent}" != "${target_probe}" ]] || return 1
    target_probe="${parent}"
  done
  [[ -d "${target_probe}" ]] || { echo "check-build-space.sh: target ancestor is not a directory: ${target_probe}" >&2; return 1; }
}

cd "${REPO_ROOT}"
df -h / >&2
# Keep a suffix so command substitution cannot strip newlines from the target path.
target="$(cargo metadata --locked --no-deps --format-version 1 \
  | python3 -I -c 'import json,sys; from pathlib import Path; sys.stdout.write(str(Path(json.load(sys.stdin)["target_directory"]).resolve()) + ".")')"
target="${target%.}"
if [[ -e "${target}" && ! -d "${target}" ]]; then
  echo "check-build-space.sh: target is not a directory: ${target}" >&2
  exit 1
fi
if [[ -d "${target}" ]]; then
  target_size="$(du -sk "${target}" | awk 'NR == 1 { print $1 }')"
  [[ "${target_size}" =~ ^[0-9]+$ ]] || { echo "check-build-space.sh: cannot measure target: ${target}" >&2; exit 1; }
  (( target_size <= estimate )) || estimate="${target_size}"
fi
root_free="$(available_kib /)"
target_parent "${target}"
target_free="$(available_kib "${target_probe}")"
if (( root_free < MIN_FREE_KIB || target_free < MIN_FREE_KIB + estimate )); then
  echo "check-build-space.sh: insufficient space for ${target}; cleaning this package's build artifacts" >&2
  if ! cargo clean --locked --package xcalygraph --target-dir "${target}"; then
    echo "check-build-space.sh: build aborted; Cargo could not safely clean ${target}" >&2
    exit 1
  fi
  df -h / >&2
  root_free="$(available_kib /)"
  target_parent "${target}"
  target_free="$(available_kib "${target_probe}")"
  if (( root_free < MIN_FREE_KIB || target_free < MIN_FREE_KIB + estimate )); then
    echo "check-build-space.sh: build aborted; free at least 2 GiB plus the estimated build size for ${target}" >&2
    exit 1
  fi
fi
