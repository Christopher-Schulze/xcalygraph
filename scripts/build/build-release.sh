#!/usr/bin/env bash
# Build release archives and checksums for GitHub Releases (docs/spec.md section 10).
# Usage: scripts/build/build-release.sh [TARGET...]
#   TARGET is a Rust target triple (default: the host). Writes dist/xcalygraph-<version>-<os>-<arch>.tar.gz
#   (binary, licenses, third-party license notices) and dist/SHA256SUMS.
# Requires: cargo, jq, tar, and shasum or sha256sum.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly DIST="${REPO_ROOT}/dist"

cd "${REPO_ROOT}"
command -v jq >/dev/null || { echo "build-release.sh: jq missing" >&2; exit 1; }
version="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "xcalygraph") | .version')"
targets=("$@")
if (( ${#targets[@]} == 0 )); then
  targets=("$(rustc -vV | awk '/^host:/ { print $2 }')")
fi

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

rm -rf "${DIST}"
mkdir -p "${DIST}"
third_party > "${DIST}/THIRD-PARTY-LICENSES.txt"
for target in "${targets[@]}"; do
  cargo build --release --locked --target "${target}"
  name="xcalygraph-${version}-$(platform "${target}")"
  stage="${DIST}/${name}"
  mkdir -p "${stage}"
  cp "target/${target}/release/xcalygraph" "${stage}/xcalygraph"
  cp LICENSE-MIT LICENSE-APACHE "${DIST}/THIRD-PARTY-LICENSES.txt" "${stage}/"
  tar -C "${DIST}" -czf "${DIST}/${name}.tar.gz" "${name}"
  rm -rf "${stage}"
done
(cd "${DIST}" && checksum ./*.tar.gz | sed 's| \./| |' > SHA256SUMS)
rm -f "${DIST}/THIRD-PARTY-LICENSES.txt"
ls -l "${DIST}"
