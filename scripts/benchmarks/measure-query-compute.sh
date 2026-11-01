#!/usr/bin/env bash
# Measure in-process find/show compute on an immutable corpus snapshot with an isolated HOME.
# Usage: XCALYGRAPH_BENCH_OPT_LEVEL=0|3 scripts/benchmarks/measure-query-compute.sh ROOT QUESTIONS.tsv [SAMPLES]
# Questions have the columns question and expected path::name. Results remain under .cache/.
set -euo pipefail

if (( $# < 2 || $# > 3 )); then
  echo "usage: measure-query-compute.sh ROOT QUESTIONS.tsv [SAMPLES]" >&2
  exit 2
fi
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
readonly REPO_ROOT
readonly SAMPLES="${3:-10}"
readonly OPT_LEVEL="${XCALYGRAPH_BENCH_OPT_LEVEL:-0}"
if [[ ! "${SAMPLES}" =~ ^[0-9]{1,4}$ ]] || (( 10#${SAMPLES} < 1 || 10#${SAMPLES} > 1000 )); then
  echo "measure-query-compute.sh: samples must be 1 to 1000" >&2
  exit 2
fi
[[ "${OPT_LEVEL}" == 0 || "${OPT_LEVEL}" == 3 ]] || {
  echo "measure-query-compute.sh: XCALYGRAPH_BENCH_OPT_LEVEL must be 0 or 3" >&2
  exit 2
}
[[ -d "$1" && -f "$2" ]] || {
  echo "measure-query-compute.sh: corpus or question file missing" >&2
  exit 2
}
ROOT="$(cd "$1" && pwd -P)"
QUESTIONS="$(cd "$(dirname "$2")" && pwd -P)/$(basename "$2")"
readonly ROOT QUESTIONS
readonly RUSTUP_DIR="${RUSTUP_HOME:-${HOME}/.rustup}"
readonly CARGO_DIR="${CARGO_HOME:-${HOME}/.cargo}"
if [[ -L "${REPO_ROOT}/.cache" || -L "${REPO_ROOT}/.cache/benchmarks" ]]; then
  echo "measure-query-compute.sh: benchmark cache must not be a symlink" >&2
  exit 1
fi
if ! bash "${REPO_ROOT}/scripts/utils/verification-lock.sh"; then
  exec bash "${REPO_ROOT}/scripts/utils/guard-benchmark.sh" \
    bash "${REPO_ROOT}/scripts/benchmarks/measure-query-compute.sh" "${ROOT}" "${QUESTIONS}" "${SAMPLES}"
fi
mkdir -p "${REPO_ROOT}/.cache/benchmarks"
WORK="$(mktemp -d "${REPO_ROOT}/.cache/benchmarks/query.XXXXXXXX")"
readonly WORK
echo "evidence: ${WORK}"
mkdir "${WORK}/home"
cp "${QUESTIONS}" "${WORK}/questions.tsv"
cd "${REPO_ROOT}"

bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
BINARY="$(RUSTFLAGS="-Dwarnings" cargo build --locked --bin xcalygraph --message-format=json -q \
  | bash "${REPO_ROOT}/scripts/utils/cargo-artifact.sh")"
readonly BINARY
snapshot="$(env HOME="${WORK}/home" bash scripts/utils/prepare-corpus.sh "${ROOT}" "${REPO_ROOT}/.cache/corpus-snapshots")"
readonly snapshot
env HOME="${WORK}/home" "${BINARY}" index -C "${snapshot}" --index-dir "${WORK}/index" | tee "${WORK}/index.txt"
stores=("${WORK}"/index/*)
(( ${#stores[@]} == 1 )) && [[ -d "${stores[0]}" ]] || {
  echo "measure-query-compute.sh: expected one index" >&2
  exit 1
}
python3 -I - "${BINARY}" "${WORK}/questions.tsv" "${snapshot}/../snapshot.json" \
  src/app/query/find.rs src/app/query/ctx.rs src/app/query/names.rs src/app/query/show.rs \
  src/app/query/mod.rs src/app/query/output.rs src/app/query/benchmark.rs src/domain/search.rs \
  src/infra/store/derived.rs > "${WORK}/identity.tsv" <<'PYTHON'
import hashlib
import json
from pathlib import Path
import sys
for name in sys.argv[1:]:
    path = Path(name)
    label = json.dumps(name) if any(ord(char) < 32 or ord(char) == 127 for char in name) else name
    print(hashlib.sha256(path.read_bytes()).hexdigest(), label, sep="\t")
PYTHON
printf 'test-profile-package-opt-level\t%s\n' "${OPT_LEVEL}" >> "${WORK}/identity.tsv"
test_profile_args=(--config "profile.test.package.xcalygraph.opt-level=${OPT_LEVEL}")
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
env HOME="${WORK}/home" RUSTUP_HOME="${RUSTUP_DIR}" CARGO_HOME="${CARGO_DIR}" RUSTFLAGS="-Dwarnings" \
  XCALYGRAPH_BENCH_STORE="${stores[0]}" XCALYGRAPH_BENCH_QUESTIONS="${WORK}/questions.tsv" \
  XCALYGRAPH_BENCH_SAMPLES="${SAMPLES}" cargo test -v "${test_profile_args[@]}" --locked --bin xcalygraph \
  --message-format=json measure_query_compute -- --ignored --nocapture 2> "${WORK}/cargo.log" \
  | bash "${REPO_ROOT}/scripts/utils/cargo-artifact.sh" --test --passthrough 3> "${WORK}/test-binary.path" \
  | tee "${WORK}/results.txt"
python3 -I - "${WORK}/test-binary.path" >> "${WORK}/identity.tsv" <<'PYTHON'
import hashlib
import json
from pathlib import Path
import sys
name = Path(sys.argv[1]).read_text()
if not name.endswith("\n"):
    raise ValueError("Cargo did not report the benchmark test binary")
name = name[:-1]
label = json.dumps(name) if any(ord(char) < 32 or ord(char) == 127 for char in name) else name
print(hashlib.sha256(Path(name).read_bytes()).hexdigest(), label, sep="\t")
PYTHON
env HOME="${WORK}/home" bash scripts/utils/prepare-corpus.sh --verify "${snapshot}" >/dev/null
echo "evidence: ${WORK}"
( set -o noclobber; printf 'xcalygraph-cache-v1\n' > "${WORK}/.xcalygraph-disposable" )
