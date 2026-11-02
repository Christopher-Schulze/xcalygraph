#!/usr/bin/env bash
# Build release archives and checksums for GitHub Releases (docs/spec.md section 10).
# Usage: scripts/build/build-release.sh [TARGET...]
#   TARGET is a Rust target triple (default: the host). Writes dist/xcalygraph-<version>-<os>-<arch>.tar.gz
#   (binary, licenses, third-party license notices) and dist/SHA256SUMS.
# Requires: bash, cargo, git, jq, python3, rustc, awk, tar, and shasum or sha256sum.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
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
    | python3 -I -c '
import json
import os
import shutil
import stat
import sys

def declared_file(directory, value):
    current = os.sep if os.path.isabs(value) else directory
    for component in value.split(os.sep):
        if component in ("", "."):
            continue
        current = os.path.join(current, component)
        if stat.S_ISLNK(os.stat(current, follow_symlinks=False).st_mode):
            raise OSError("declared license path contains a symlink")
    info = os.stat(current, follow_symlinks=False)
    if not stat.S_ISREG(info.st_mode):
        raise OSError("declared license is not a regular file")
    return current, (info.st_dev, info.st_ino)

def write_license(path, name):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        info = os.fstat(source.fileno())
        if not stat.S_ISREG(info.st_mode):
            raise OSError("license entry is no longer a regular file")
        output.write(b"---- " + os.fsencode(name) + b"\n")
        shutil.copyfileobj(source, output)
        output.write(b"\n")
        return info.st_dev, info.st_ino

packages = set()
for package in json.load(sys.stdin)["packages"]:
    if package["name"] == "xcalygraph":
        continue
    manifest = package["manifest_path"]
    if os.path.basename(manifest) != "Cargo.toml":
        raise ValueError("unexpected Cargo manifest path")
    license_name = package["license"] if package["license"] is not None else "see files"
    license_file = package["license_file"]
    packages.add((package["name"], package["version"], license_name, os.path.dirname(manifest),
                  license_file is not None, license_file or ""))

output = sys.stdout.buffer
for name, version, license_name, directory, has_declared, declaration in sorted(
        packages, key=lambda row: (tuple(map(os.fsencode, row[:4])), row[4], os.fsencode(row[5]))):
    if not stat.S_ISDIR(os.stat(directory, follow_symlinks=False).st_mode):
        raise OSError("dependency directory is not a regular directory")
    declared = declared_file(directory, declaration) if has_declared else None
    declared_seen = False
    output.write(b"==== " + os.fsencode(name) + b" " + os.fsencode(version)
                 + b" (" + os.fsencode(license_name) + b")\n")
    with os.scandir(directory) as entries:
        files = sorted((entry for entry in entries
                        if os.fsencode(entry.name).lower().startswith((b"license", b"copying", b"notice"))),
                       key=lambda entry: os.fsencode(entry.name))
    for entry in files:
        if not stat.S_ISREG(entry.stat(follow_symlinks=False).st_mode):
            continue
        identity = write_license(entry.path, entry.name)
        if declared is not None and identity == declared[1]:
            declared_seen = True
    if declared is not None and not declared_seen:
        write_license(declared[0], os.path.basename(declaration))
'
}

checksum() {
  if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}

check_requirements() {
  local tool
  for tool in bash cargo git jq python3 rustc awk tar; do
    command -v "${tool}" >/dev/null || {
      echo "build-release.sh: required command missing: ${tool}" >&2
      return 1
    }
  done
  if ! command -v sha256sum >/dev/null && ! command -v shasum >/dev/null; then
    echo "build-release.sh: required command missing: sha256sum or shasum" >&2
    return 1
  fi
  return 0
}

package_release() {
  local stage="$1" name="$2" binary="$3"
  mkdir "${stage}/${name}" || return 1
  cp "${binary}" "${stage}/${name}/xcalygraph" || return 1
  cp "${REPO_ROOT}/LICENSE" "${stage}/THIRD-PARTY-LICENSES.txt" "${stage}/${name}/" || return 1
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
  (cd "${stage}" && checksum "${archives[@]}" > SHA256SUMS) || return 1
  for name in "${archives[@]}" SHA256SUMS; do
    if ! ln "${stage}/${name}" "${destination}/${name}"; then
      echo "build-release.sh: could not publish ${name}; earlier outputs remain in ${destination}" >&2
      return 1
    fi
  done
}

stage_releases() {
  local stage="$1" expected_version="$2" profile="$3" target binary name version
  shift 3
  (( $# > 0 )) || { echo "build-release.sh: no targets to stage" >&2; return 1; }
  local -a build_args=()
  case "${profile}" in
    --debug) ;;
    --release) build_args+=(--release) ;;
    *) echo "build-release.sh: invalid build profile" >&2; return 1 ;;
  esac
  if ! bash "${REPO_ROOT}/scripts/utils/verification-lock.sh"; then
    echo "build-release.sh: verification lock is required" >&2
    return 1
  fi
  version="$(cargo metadata --locked --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "xcalygraph") | .version')" || return 1
  [[ "${version}" == "${expected_version}" ]] || { echo "build-release.sh: version changed before build" >&2; return 1; }
  ( set -o noclobber; third_party > "${stage}/THIRD-PARTY-LICENSES.txt" ) || return 1
  for target in "$@"; do
    name="xcalygraph-${version}-$(platform "${target}")" || return 1
    bash "${REPO_ROOT}/scripts/utils/check-build-space.sh" || return 1
    binary="$(cargo build ${build_args[@]+"${build_args[@]}"} --locked --bin xcalygraph --target "${target}" --message-format=json \
      | bash "${REPO_ROOT}/scripts/utils/cargo-artifact.sh")" || return 1
    package_release "${stage}" "${name}" "${binary}" || return 1
  done
}

build_archives() (
  local profile="$1" version target platform_name stage
  shift
  local -a targets=("$@") archives=()
  cd "${REPO_ROOT}" || return 1
  check_requirements || return 1
  version="$(cargo metadata --locked --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "xcalygraph") | .version')" || return 1
  if (( ${#targets[@]} == 0 )); then
    targets=("$(rustc -vV | awk '/^host:/ { print $2 }')")
  fi
  [[ ! -L "${DIST}" ]] || { echo "build-release.sh: destination is a symlink: ${DIST}" >&2; return 1; }
  mkdir -p "${DIST}" || return 1
  for target in "${targets[@]}"; do
    platform_name="$(platform "${target}")" || return 1
    archives+=("xcalygraph-${version}-${platform_name}.tar.gz")
  done
  check_outputs "${DIST}" "${archives[@]}" || return 1
  stage="$(mktemp -d "${DIST}/.xcalygraph-release.XXXXXXXX")" || return 1
  trap 'rm -rf -- "${stage}"' EXIT
  bash "${REPO_ROOT}/scripts/utils/guard-benchmark.sh" bash -c \
    'source "$1"; shift; stage_releases "$@"' release-stage \
    "${REPO_ROOT}/scripts/build/build-release.sh" "${stage}" "${version}" "${profile}" "${targets[@]}" || return 1
  publish_releases "${stage}" "${DIST}" "${archives[@]}" || return 1
  ls -l "${DIST}"
)

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  return 0
fi
build_archives --release "$@"
