#!/usr/bin/env bash
# Check the root, Cargo target and intermediate filesystems before a build.
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
  value="$(df -Pk "$1" | awk 'NR == 2 { print $4 }')" || {
    echo "check-build-space.sh: cannot measure free space: $1" >&2
    return 1
  }
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
  [[ -d "${target_probe}" ]] || { echo "check-build-space.sh: directory ancestor is not a directory: ${target_probe}" >&2; return 1; }
}

directory_size() {
  local path="$1" label="$2" size=0
  if [[ -e "${path}" && ! -d "${path}" ]]; then
    echo "check-build-space.sh: ${label} is not a directory: ${path}" >&2
    return 1
  fi
  if [[ -d "${path}" ]]; then
    size="$(du -sk "${path}" | awk 'NR == 1 { print $1 }')" || {
      echo "check-build-space.sh: cannot measure ${label}: ${path}" >&2
      return 1
    }
    [[ "${size}" =~ ^[0-9]+$ ]] || { echo "check-build-space.sh: cannot measure ${label}: ${path}" >&2; return 1; }
  fi
  printf '%s\n' "${size}"
}

check_filesystems() {
  root_free="$(available_kib /)"
  target_parent "${target}"
  target_free="$(available_kib "${target_probe}")"
  if [[ "${build}" == "${target}" ]]; then
    build_free="${target_free}"
  else
    target_parent "${build}"
    build_free="$(available_kib "${target_probe}")"
  fi
}

cd "${REPO_ROOT}"
df -h / >&2
cargo_metadata="$(cargo metadata --locked --no-deps --format-version 1)"
# Keep a suffix so command substitution cannot strip newlines from the paths.
target="$(python3 -I -c 'import json,sys; from pathlib import Path; sys.stdout.write(str(Path(json.load(sys.stdin)["target_directory"]).resolve()) + ".")' <<< "${cargo_metadata}")"
build="$(python3 -I -c 'import json,sys; from pathlib import Path; sys.stdout.write(str(Path(json.load(sys.stdin)["build_directory"]).resolve()) + ".")' <<< "${cargo_metadata}")"
target="${target%.}"
build="${build%.}"
target_size="$(directory_size "${target}" target)"
if [[ "${build}" == "${target}" ]]; then
  build_size="${target_size}"
else
  build_size="$(directory_size "${build}" 'build directory')"
fi
if [[ "${target}" == / || "${build}" == / || "${target}" == "${build}" \
      || "${target}" == "${build}/"* || "${build}" == "${target}/"* ]]; then
  if (( target_size >= build_size )); then
    combined_size="${target_size}"
  else
    combined_size="${build_size}"
  fi
else
  combined_size=$((target_size + build_size))
fi
(( combined_size <= estimate )) || estimate="${combined_size}"
check_filesystems
if (( root_free < MIN_FREE_KIB || target_free < MIN_FREE_KIB + estimate || build_free < MIN_FREE_KIB + estimate )); then
  echo "check-build-space.sh: insufficient space for ${target}; cleaning this package's build artifacts (intermediates: ${build})" >&2
  if ! cargo clean --locked --package xcalygraph --target-dir "${target}"; then
    echo "check-build-space.sh: build aborted; Cargo could not safely clean ${target}" >&2
    exit 1
  fi
  df -h / >&2
  check_filesystems
  if (( root_free < MIN_FREE_KIB || target_free < MIN_FREE_KIB + estimate || build_free < MIN_FREE_KIB + estimate )); then
    echo "check-build-space.sh: build aborted; free at least 2 GiB plus the estimated build size for ${target} and ${build}" >&2
    exit 1
  fi
fi
