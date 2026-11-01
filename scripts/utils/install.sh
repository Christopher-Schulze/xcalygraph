#!/usr/bin/env bash
# Install xcalygraph from a GitHub release: verified binary in ~/.local/bin, agent skill in
# ~/.agents/skills/xcalygraph, linked for Claude Code when ~/.claude exists.
# Usage (attached to every release, never fetched from main):
#   curl -fsSL https://github.com/Christopher-Schulze/xcalygraph/releases/latest/download/install.sh | bash
#   bash install.sh [--version X.Y.Z] [--no-skill]
#   XCALYGRAPH_INSTALL_DIR=DIR   install directory (default ~/.local/bin)
#   XCALYGRAPH_RELEASE_BASE=URL  release download base (default GitHub; file:// for tests)
# Verifies the archive against SHA256SUMS and smoke-tests the binary before it replaces an
# existing one. Shell profiles are never edited: a missing PATH entry is reported.
# Requires: curl, tar, and shasum or sha256sum.
set -euo pipefail

readonly REPOSITORY="Christopher-Schulze/xcalygraph"
readonly RELEASE_BASE="${XCALYGRAPH_RELEASE_BASE:-https://github.com/${REPOSITORY}/releases/download}"
readonly BIN_DIR="${XCALYGRAPH_INSTALL_DIR:-${HOME}/.local/bin}"

say() { printf '>> %s\n' "$1" >&2; }
die() { printf 'install.sh: %s\n' "$1" >&2; exit 1; }

version=""
skill=1
while (( $# > 0 )); do
  case "$1" in
    --version) [[ $# -ge 2 ]] || die "--version needs a value"; version="${2#v}"; shift 2 ;;
    --version=*) version="${1#--version=}"; version="${version#v}"; shift ;;
    --no-skill) skill=0; shift ;;
    -h | --help) sed -n '2,12p' "$0" 2>/dev/null || true; exit 0 ;;
    *) die "unknown argument $1 (use --version X.Y.Z or --no-skill)" ;;
  esac
done

case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) die "unsupported system $(uname -s)" ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch=arm64 ;;
  x86_64 | amd64) arch=amd64 ;;
  *) die "unsupported architecture $(uname -m)" ;;
esac

if [[ -z "${version}" ]]; then
  say "resolving the latest release"
  version="$(curl -fsSL "https://api.github.com/repos/${REPOSITORY}/releases/latest" \
    | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -n 1)"
  [[ -n "${version}" ]] || die "no published release found"
fi
[[ "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || die "invalid version ${version}"

name="xcalygraph-${version}-${os}-${arch}"
base="${RELEASE_BASE}/v${version}"
work="$(mktemp -d)"
temporary=""
trap '[[ -z "${temporary}" ]] || rm -f -- "${temporary}"; rm -rf -- "${work}"' EXIT

say "downloading ${name}.tar.gz"
curl -fSL --progress-bar -o "${work}/${name}.tar.gz" "${base}/${name}.tar.gz" \
  || die "download failed: ${base}/${name}.tar.gz"
curl -fsSL -o "${work}/SHA256SUMS" "${base}/SHA256SUMS" || die "download failed: ${base}/SHA256SUMS"

say "verifying SHA-256"
expected="$(awk -v file="${name}.tar.gz" '$2 == file || $2 == "*" file { print $1 }' "${work}/SHA256SUMS")"
if command -v sha256sum >/dev/null; then
  actual="$(sha256sum "${work}/${name}.tar.gz" | awk '{ print $1 }')"
else
  actual="$(shasum -a 256 "${work}/${name}.tar.gz" | awk '{ print $1 }')"
fi
[[ -n "${expected}" ]] || die "SHA256SUMS lists no ${name}.tar.gz"
[[ "${expected}" == "${actual}" ]] || die "checksum mismatch for ${name}.tar.gz"

tar -C "${work}" -xzf "${work}/${name}.tar.gz"
candidate="${work}/${name}/xcalygraph"
[[ -x "${candidate}" ]] || die "archive contains no xcalygraph binary"

say "smoke test"
reported="$("${candidate}" version 2>/dev/null | head -n 1 || true)"
[[ "${reported}" == "xcalygraph ${version}" ]] \
  || die "downloaded binary failed its smoke test (reported '${reported}'); nothing was replaced"

if (( skill )); then
  # The binary installs itself, the skill and the Claude Code link, and reports PATH problems.
  XCALYGRAPH_INSTALL_DIR="${BIN_DIR}" "${candidate}" install
else
  mkdir -p "${BIN_DIR}"
  [[ ! -d "${BIN_DIR}/xcalygraph" ]] || die "installation target is a directory: ${BIN_DIR}/xcalygraph"
  temporary="$(mktemp "${BIN_DIR}/.xcalygraph.new.XXXXXXXX")" || die "cannot create a temporary binary in ${BIN_DIR}"
  install -m 0755 "${candidate}" "${temporary}"
  mv "${temporary}" "${BIN_DIR}/xcalygraph"
  temporary=""
  echo "binary ${BIN_DIR}/xcalygraph installed"
  case ":${PATH}:" in
    *":${BIN_DIR}:"*) ;;
    *) echo "! ${BIN_DIR} is not on PATH; add: export PATH=\"${BIN_DIR}:\$PATH\"" ;;
  esac
fi
say "xcalygraph ${version} installed"
