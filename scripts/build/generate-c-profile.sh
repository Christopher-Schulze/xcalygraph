#!/usr/bin/env bash
# Train the profile of the C parser core and grammars on pinned corpora (aarch64 macOS, Apple clang).
# Usage: scripts/build/generate-c-profile.sh
#   Writes scripts/build/c-parser.profdata, used by scripts/build/build-release.sh on this target
#   only. Rust code and other targets stay untouched. A stale profile loses gain, not correctness:
#   run this after a tree-sitter or grammar update.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
cd "${REPO_ROOT}"
bash scripts/utils/check-build-space.sh
work="$(mktemp -d "${REPO_ROOT}/.cache/c-profile.XXXXXXXX")"
trap 'rm -rf -- "${work}"' EXIT
mkdir "${work}/raw"
CFLAGS_aarch64_apple_darwin="-fprofile-instr-generate" \
  RUSTFLAGS="-C link-arg=-fprofile-instr-generate" CARGO_TARGET_DIR="${work}/target" \
  cargo build --release --locked --bin xcalygraph
while IFS=$'\t' read -r name _ _ root; do
  [[ "${name}" =~ ^(hono|ripgrep|httpx|cobra|gson|laravel\.io|axios)$ ]] || continue
  LLVM_PROFILE_FILE="${work}/raw/%p.profraw" XCALYGRAPH_INDEX_DIR="${work}/index" \
    "${work}/target/release/xcalygraph" index -C "${root}" > /dev/null
  rm -rf -- "${work}/index"
done < .cache/edge-baseline/roots.tsv
xcrun llvm-profdata merge -o scripts/build/c-parser.profdata "${work}"/raw/*.profraw
ls -l scripts/build/c-parser.profdata
