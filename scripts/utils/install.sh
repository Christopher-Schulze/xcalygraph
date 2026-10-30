#!/usr/bin/env bash
# Install the latest xcalygraph release to ~/.local/bin and install its agent skill.
# Usage: curl -fsSL https://raw.githubusercontent.com/Christopher-Schulze/xcalygraph/main/scripts/utils/install.sh | bash
#   Verifies the archive against SHA256SUMS. Links the skill for Claude Code when ~/.claude exists.
# Requires: curl, tar, and shasum or sha256sum.
set -euo pipefail

readonly REPOSITORY="Christopher-Schulze/xcalygraph"
readonly BIN_DIR="${HOME}/.local/bin"

case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) echo "install.sh: unsupported system $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64 | aarch64) arch=arm64 ;;
  x86_64 | amd64) arch=amd64 ;;
  *) echo "install.sh: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac

tag="$(curl -fsSL "https://api.github.com/repos/${REPOSITORY}/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
[[ -n "${tag}" ]] || { echo "install.sh: no release found" >&2; exit 1; }
version="${tag#v}"
name="xcalygraph-${version}-${os}-${arch}"
base="https://github.com/${REPOSITORY}/releases/download/${tag}"

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT
curl -fsSL -o "${work}/${name}.tar.gz" "${base}/${name}.tar.gz"
curl -fsSL -o "${work}/SHA256SUMS" "${base}/SHA256SUMS"
expected="$(awk -v file="${name}.tar.gz" '$2 == file || $2 == "*" file { print $1 }' "${work}/SHA256SUMS")"
if command -v sha256sum >/dev/null; then
  actual="$(sha256sum "${work}/${name}.tar.gz" | awk '{ print $1 }')"
else
  actual="$(shasum -a 256 "${work}/${name}.tar.gz" | awk '{ print $1 }')"
fi
[[ -n "${expected}" && "${expected}" == "${actual}" ]] || { echo "install.sh: checksum mismatch for ${name}.tar.gz" >&2; exit 1; }

tar -C "${work}" -xzf "${work}/${name}.tar.gz"
mkdir -p "${BIN_DIR}"
install -m 0755 "${work}/${name}/xcalygraph" "${BIN_DIR}/xcalygraph.new"
mv "${BIN_DIR}/xcalygraph.new" "${BIN_DIR}/xcalygraph"
if [[ -d "${HOME}/.claude" ]]; then
  "${BIN_DIR}/xcalygraph" install --link claude
else
  "${BIN_DIR}/xcalygraph" install
fi
case ":${PATH}:" in
  *":${BIN_DIR}:"*) ;;
  *) echo "add ${BIN_DIR} to PATH to run xcalygraph" ;;
esac
