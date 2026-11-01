#!/usr/bin/env bash
# Measure in-process find/show compute on an immutable corpus snapshot with an isolated HOME.
# Usage: XCALYGRAPH_BENCH_OPT_LEVEL=0|3 scripts/benchmarks/measure-query-compute.sh ROOT QUESTIONS.tsv [SAMPLES]
# Questions have the columns question and expected path::name. Results remain under .cache/.
set -euo pipefail

if (( $# < 2 || $# > 3 )); then
  echo "usage: measure-query-compute.sh ROOT QUESTIONS.tsv [SAMPLES]" >&2
  exit 2
fi
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
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
readonly BINARY="${REPO_ROOT}/target/debug/xcalygraph"
readonly RUSTUP_DIR="${RUSTUP_HOME:-${HOME}/.rustup}"
readonly CARGO_DIR="${CARGO_HOME:-${HOME}/.cargo}"
mkdir -p "${REPO_ROOT}/.cache/benchmarks"
WORK="$(mktemp -d "${REPO_ROOT}/.cache/benchmarks/query.XXXXXXXX")"
readonly WORK
echo "evidence: ${WORK}"
mkdir "${WORK}/home"
cp "${QUESTIONS}" "${WORK}/questions.tsv"
cd "${REPO_ROOT}"

check_build_space() {
  local available estimate=0
  df -h / >&2
  available="$(df -Pk / | awk 'NR == 2 { print $4 }')"
  [[ ! -d target ]] || estimate="$(du -sk target | awk '{ print $1 }')"
  if (( available < 2 * 1024 * 1024 + estimate )); then
    cargo clean
    df -h / >&2
    available="$(df -Pk / | awk 'NR == 2 { print $4 }')"
    if (( available < 2 * 1024 * 1024 + estimate )); then
      echo "Build aborted: free at least 2 GiB plus the estimated build output." >&2
      exit 1
    fi
  fi
}

check_build_space
RUSTFLAGS="-Dwarnings" cargo build --locked -q
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
from pathlib import Path
import sys
for name in sys.argv[1:]:
    path = Path(name)
    print(hashlib.sha256(path.read_bytes()).hexdigest(), name, sep="\t")
PYTHON
printf 'test-profile-package-opt-level\t%s\n' "${OPT_LEVEL}" >> "${WORK}/identity.tsv"
test_profile_args=(--config "profile.test.package.xcalygraph.opt-level=${OPT_LEVEL}")
check_build_space
env HOME="${WORK}/home" RUSTUP_HOME="${RUSTUP_DIR}" CARGO_HOME="${CARGO_DIR}" RUSTFLAGS="-Dwarnings" \
  XCALYGRAPH_BENCH_STORE="${stores[0]}" XCALYGRAPH_BENCH_QUESTIONS="${WORK}/questions.tsv" \
  XCALYGRAPH_BENCH_SAMPLES="${SAMPLES}" cargo test -v "${test_profile_args[@]}" --locked --bin xcalygraph measure_query_compute -- --ignored --nocapture 2> "${WORK}/cargo.log" \
  | tee "${WORK}/results.txt"
python3 -I - "${WORK}/cargo.log" >> "${WORK}/identity.tsv" <<'PYTHON'
import hashlib
from pathlib import Path
import shlex
import sys
for line in reversed(Path(sys.argv[1]).read_text().splitlines()):
    if "Running `" in line and " measure_query_compute " in line:
        command = line.split("Running `", 1)[1].rsplit("`", 1)[0]
        binary = Path(shlex.split(command)[0])
        print(hashlib.sha256(binary.read_bytes()).hexdigest(), binary, sep="\t")
        break
else:
    raise ValueError("Cargo did not report the benchmark test binary")
PYTHON
env HOME="${WORK}/home" bash scripts/utils/prepare-corpus.sh --verify "${snapshot}" >/dev/null
echo "evidence: ${WORK}"
