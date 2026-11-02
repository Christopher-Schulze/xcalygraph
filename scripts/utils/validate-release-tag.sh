#!/usr/bin/env bash
# Require a release tag to encode the Cargo package version exactly.
# Usage: scripts/utils/validate-release-tag.sh TAG CARGO_VERSION
set -euo pipefail

(( $# == 2 )) || {
  echo "usage: validate-release-tag.sh TAG CARGO_VERSION" >&2
  exit 2
}

tag="$1"
version="$2"
[[ -n "${tag}" && -n "${version}" ]] || {
  echo "validate-release-tag.sh: tag and Cargo package version must be nonempty" >&2
  exit 2
}

expected="v${version}"
[[ "${tag}" == "${expected}" ]] || {
  printf 'validate-release-tag.sh: tag %s does not match Cargo package version %s (expected %s)\n' \
    "${tag}" "${version}" "${expected}" >&2
  exit 1
}
