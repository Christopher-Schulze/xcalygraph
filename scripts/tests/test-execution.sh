#!/usr/bin/env bash
# Run the gate checks inside the owned, content-guarded verification target.
# Usage: scripts/tests/test-execution.sh WORK_DIRECTORY product|all
set -euo pipefail
(( $# == 2 )) && [[ "$2" == product || "$2" == all ]] || {
  echo "usage: test-execution.sh WORK_DIRECTORY product|all" >&2
  exit 2
}
readonly work="$1" mode="$2"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
readonly REPO_ROOT
cd "${REPO_ROOT}"
cargo fmt --all --check
check_feature_subset() {
  bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
  RUSTFLAGS="-Dwarnings" cargo check --locked "$@" --bin xcalygraph
}
check_feature_subset --no-default-features
check_feature_subset --no-default-features --features lang-bash
check_feature_subset --no-default-features --features lang-javascript
check_feature_subset --no-default-features --features lang-typescript
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
cargo clippy --all-targets --locked -- -D warnings
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
cargo test --locked
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
cargo build --locked --bin xcalygraph --message-format=json > "${work}/build.jsonl"
binary="$(python3 -I - "${work}/build.jsonl" <<'PYTHON'
import json
import sys

artifacts = [json.loads(line) for line in open(sys.argv[1])]
executables = [item["executable"] for item in artifacts
               if item.get("reason") == "compiler-artifact"
               and item["target"]["name"] == "xcalygraph"
               and "bin" in item["target"]["kind"] and item.get("executable")]
if len(executables) != 1:
    raise SystemExit("test.sh: Cargo did not report exactly one xcalygraph binary")
print(executables[0])
PYTHON
)"
[[ "${mode}" == all ]] || exit 0
bash "${REPO_ROOT}/scripts/tests/test-prune-cache.sh"
bash "${REPO_ROOT}/scripts/tests/test-edge-oracle.sh"
bash "${REPO_ROOT}/scripts/tests/test-agent-benchmark-safety.sh" "${binary}"
bash "${REPO_ROOT}/scripts/tests/test-agent-benchmark-questions.sh"
bash "${REPO_ROOT}/scripts/tests/test-benchmark-inputs.sh"
bash "${REPO_ROOT}/scripts/tests/test-benchmark-input-paths.sh"
bash "${REPO_ROOT}/scripts/tests/test-performance-benchmark-corpora.sh" "${binary}"
bash "${REPO_ROOT}/scripts/tests/test-benchmark-output-writer.sh" "${binary}"
bash "${REPO_ROOT}/scripts/tests/test-performance-measurement-paths.sh"
bash "${REPO_ROOT}/scripts/tests/test-performance-source-paths.sh" "${binary}"
bash "${REPO_ROOT}/scripts/tests/test-evaluation-relative-corpus.sh"
bash "${REPO_ROOT}/scripts/tests/test-build-space.sh"
bash "${REPO_ROOT}/scripts/tests/test-hyperfine-args.sh"
bash "${REPO_ROOT}/scripts/tests/test-cargo-artifact.sh"
bash "${REPO_ROOT}/scripts/tests/test-verification-target.sh"
bash "${REPO_ROOT}/scripts/tests/test-verification-source.sh"
bash "${REPO_ROOT}/scripts/tests/test-verification-inputs.sh"
bash "${REPO_ROOT}/scripts/tests/test-guard-benchmark.sh"
bash "${REPO_ROOT}/scripts/tests/test-depth-comparison-provenance.sh"
bash "${REPO_ROOT}/scripts/tests/test-checkout-root.sh"
bash "${REPO_ROOT}/scripts/tests/test-benchmark-checkout-root.sh"
bash "${REPO_ROOT}/scripts/tests/test-publication-paths.sh"
bash "${REPO_ROOT}/scripts/tests/test-publication-scans.sh"
bash "${REPO_ROOT}/scripts/tests/test-publication-blobs.sh"
bash "${REPO_ROOT}/scripts/tests/test-release-provenance.sh"
bash "${REPO_ROOT}/scripts/tests/test-release-licenses.sh"
bash "${REPO_ROOT}/scripts/tests/test-cache-producers.sh" "${binary}"
bash "${REPO_ROOT}/scripts/tests/test-corpus-snapshots.sh" "${binary}"
bash "${REPO_ROOT}/scripts/tests/test-evaluation-labels.sh" "${binary}"
