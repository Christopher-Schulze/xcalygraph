#!/usr/bin/env bash
# Verify the dependency supply chain: vendored grammar checksums, licenses, sources and advisories.
# Needs the developer tools cargo-deny and cargo-audit and network access to the advisory database.
# Usage: scripts/audits/check-supply-chain.sh
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
cd "${REPO_ROOT}"

for tool in cargo-deny cargo-audit; do
  command -v "${tool}" >/dev/null || { echo "check-supply-chain.sh: ${tool} is required" >&2; exit 2; }
done

if command -v sha256sum >/dev/null; then
  (cd vendor/tree-sitter-robot && sha256sum --check --quiet SOURCE.sha256)
else
  (cd vendor/tree-sitter-robot && shasum -a 256 --check --quiet SOURCE.sha256)
fi
cargo deny --locked --log-level error check
cargo audit --deny warnings --quiet
echo "check-supply-chain.sh: clean"
