#!/usr/bin/env bash
# Build release archives and checksums for GitHub Releases (docs/spec.md section 10).
# Usage: scripts/build/build-release.sh [TARGET...]
#   TARGET is a Rust target triple (default: the host). Writes dist/xcalygraph-<version>-<os>-<arch>.tar.gz
#   (binary, licenses, third-party license notices) and dist/SHA256SUMS.
# Requires: cargo, jq, python3, tar, and shasum or sha256sum.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly DIST="${REPO_ROOT}/dist"

platform() {
  local os arch
  case "$1" in
    *apple-darwin) os=darwin ;;
    *linux*) os=linux ;;
    *) echo "build-release.sh: unsupported target $1" >&2; return 1 ;;
  esac
  case "$1" in
    aarch64-*) arch=arm64 ;;
    x86_64-*) arch=amd64 ;;
    *) echo "build-release.sh: unsupported target $1" >&2; return 1 ;;
  esac
  echo "${os}-${arch}"
}

# License notices of every bundled dependency: name, version, license, then its license files.
third_party() {
  cargo metadata --locked --format-version 1 \
    | jq -r '.packages[] | select(.name != "xcalygraph") | [.name, .version, (.license // "see files"), (.manifest_path | sub("/Cargo.toml$"; ""))] | @tsv' \
    | sort -u \
    | while IFS=$'\t' read -r name crate_version license dir; do
        printf '==== %s %s (%s)\n' "${name}" "${crate_version}" "${license}"
        find "${dir}" -maxdepth 1 -type f \( -iname 'LICENSE*' -o -iname 'COPYING*' -o -iname 'NOTICE*' \) -print \
          | sort | while IFS= read -r file; do
              printf -- '---- %s\n' "$(basename "${file}")"
              cat "${file}"
              printf '\n'
            done
      done
}

checksum() {
  if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}

package_release() {
  local stage="$1" name="$2" binary="$3"
  mkdir "${stage}/${name}"
  cp "${binary}" "${stage}/${name}/xcalygraph"
  cp "${REPO_ROOT}/LICENSE" "${stage}/THIRD-PARTY-LICENSES.txt" "${stage}/${name}/"
  tar -C "${stage}" -czf "${stage}/${name}.tar.gz" "${name}"
}

check_outputs() {
  local destination="$1" name previous
  shift
  local -a seen=()
  [[ -d "${destination}" && ! -L "${destination}" ]] || {
    echo "build-release.sh: destination is not a regular directory: ${destination}" >&2
    return 1
  }
  for name in "$@" SHA256SUMS; do
    for previous in ${seen[@]+"${seen[@]}"}; do
      [[ "${name}" != "${previous}" ]] || {
        echo "build-release.sh: duplicate output: ${name}" >&2
        return 1
      }
    done
    seen+=("${name}")
    [[ ! -e "${destination}/${name}" && ! -L "${destination}/${name}" ]] || {
      echo "build-release.sh: output already exists: ${destination}/${name}" >&2
      return 1
    }
  done
}

publish_releases() {
  local stage="$1" destination="$2" name
  shift 2
  (( $# > 0 )) || { echo "build-release.sh: no archives to publish" >&2; return 1; }
  local -a archives=("$@")
  check_outputs "${destination}" "${archives[@]}" || return 1
  (cd "${stage}" && checksum "${archives[@]}" > SHA256SUMS)
  for name in "${archives[@]}" SHA256SUMS; do
    if ! ln "${stage}/${name}" "${destination}/${name}"; then
      echo "build-release.sh: could not publish ${name}; earlier outputs remain in ${destination}" >&2
      return 1
    fi
  done
}

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  return 0
fi

cd "${REPO_ROOT}"
command -v jq >/dev/null || { echo "build-release.sh: jq missing" >&2; exit 1; }
version="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "xcalygraph") | .version')"
targets=("$@")
if (( $# == 0 )); then
  targets=("$(rustc -vV | awk '/^host:/ { print $2 }')")
fi

[[ ! -L "${DIST}" ]] || { echo "build-release.sh: destination is a symlink: ${DIST}" >&2; exit 1; }
mkdir -p "${DIST}"
archives=()
for target in "${targets[@]}"; do
  platform_name="$(platform "${target}")" || exit 1
  archives+=("xcalygraph-${version}-${platform_name}.tar.gz")
done
check_outputs "${DIST}" "${archives[@]}"
stage="$(mktemp -d "${DIST}/.xcalygraph-release.XXXXXXXX")"
trap 'rm -rf -- "${stage}"' EXIT
third_party > "${stage}/THIRD-PARTY-LICENSES.txt"
index=0
for target in "${targets[@]}"; do
  bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
  cargo build --release --locked --target "${target}"
  name="${archives[${index}]%.tar.gz}"
  package_release "${stage}" "${name}" "target/${target}/release/xcalygraph"
  index=$((index + 1))
done
publish_releases "${stage}" "${DIST}" "${archives[@]}"
ls -l "${DIST}"
