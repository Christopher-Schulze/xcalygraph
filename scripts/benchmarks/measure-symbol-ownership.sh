#!/usr/bin/env bash
# Compare release index builds for one source snapshot and a minified Rust stress file.
# Usage: measure-symbol-ownership.sh BASELINE_BINARY CANDIDATE_BINARY CORPUS_REV [RUNS]
# Requires: git, hyperfine, jq, python3, shasum.
set -euo pipefail

if (( $# < 3 || $# > 4 )); then
  echo "usage: measure-symbol-ownership.sh BASELINE_BINARY CANDIDATE_BINARY CORPUS_REV [RUNS]" >&2
  exit 2
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
readonly REPO_ROOT
readonly RUNS="${4:-7}"
readonly MIN_FREE_KIB=$((2 * 1024 * 1024))
readonly -a STRESS_SYMBOL_COUNTS=(256 1024 2048 4096 8000)

for variable in ${!GIT_@} ${!XCALYGRAPH_@}; do unset "${variable}"; done
unset BASH_ENV ENV CDPATH TAR_OPTIONS
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 LC_ALL=C

if [[ ! "${RUNS}" =~ ^[0-9]{1,2}$ ]] || (( 10#${RUNS} < 3 || 10#${RUNS} > 20 )); then
  echo "measure-symbol-ownership.sh: runs must be 3 to 20" >&2
  exit 2
fi

for tool in git hyperfine jq python3 shasum; do
  command -v "${tool}" >/dev/null || {
    echo "measure-symbol-ownership.sh: ${tool} is required" >&2
    exit 1
  }
done

if [[ -L "${REPO_ROOT}/.cache" || -L "${REPO_ROOT}/.cache/benchmarks" ]]; then
  echo "measure-symbol-ownership.sh: benchmark cache must not be a symlink" >&2
  exit 1
fi
AVAILABLE_KIB="$(df -Pk "${REPO_ROOT}" | awk 'NR == 2 { print $4 }')"
if [[ ! "${AVAILABLE_KIB}" =~ ^[0-9]+$ ]] || (( AVAILABLE_KIB < MIN_FREE_KIB + 1024 * 1024 )); then
  echo "measure-symbol-ownership.sh: requires 2 GiB reserve plus 1 GiB working headroom" >&2
  df -h "${REPO_ROOT}" >&2
  exit 1
fi

resolve_binary() {
  local resolved
  [[ ! -L "$1" ]] || {
    echo "measure-symbol-ownership.sh: binary path cannot be a symlink: $1" >&2
    return 1
  }
  resolved="$(python3 -I -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).resolve(strict=True))' "$1")" || return
  [[ -f "${resolved}" && -x "${resolved}" && ! -L "${resolved}" ]] || {
    echo "measure-symbol-ownership.sh: binary must be an executable regular file: $1" >&2
    return 1
  }
  printf '%s\n' "${resolved}"
}

BASELINE="$(resolve_binary "$1")"
CANDIDATE="$(resolve_binary "$2")"
REVISION="$(git -C "${REPO_ROOT}" rev-parse --verify --end-of-options "$3^{commit}")" || {
  echo "measure-symbol-ownership.sh: CORPUS_REV must resolve to a commit" >&2
  exit 2
}
readonly BASELINE CANDIDATE REVISION

mkdir -p "${REPO_ROOT}/.cache/benchmarks"
WORK="$(mktemp -d "${REPO_ROOT}/.cache/benchmarks/symbol-ownership.XXXXXXXX")"
readonly WORK
mkdir -m 700 "${WORK}/home" "${WORK}/temporary"
export HOME="${WORK}/home" TMPDIR="${WORK}/temporary" XDG_CONFIG_HOME="${WORK}/home/config"

cleanup() {
  local status=$?
  trap - EXIT
  case "${WORK}" in
    "${REPO_ROOT}/.cache/benchmarks/symbol-ownership."*)
      if [[ -d "${WORK}" && ! -L "${WORK}" ]]; then
        rm -rf -- "${WORK}" || status=1
      else
        echo "measure-symbol-ownership.sh: refusing to remove changed benchmark path" >&2
        status=1
      fi
      ;;
    *)
      echo "measure-symbol-ownership.sh: refusing to remove unexpected benchmark path" >&2
      status=1
      ;;
  esac
  exit "${status}"
}
trap cleanup EXIT

mkdir -m 700 "${WORK}/realistic"
git -C "${REPO_ROOT}" archive --format=tar "${REVISION}" > "${WORK}/corpus.tar"
python3 -I - "${WORK}/corpus.tar" "${WORK}/realistic" "${WORK}" "${STRESS_SYMBOL_COUNTS[@]}" <<'PYTHON'
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile
import sys

archive, realistic, work = map(Path, sys.argv[1:4])
symbol_counts = [int(value) for value in sys.argv[4:]]
with tarfile.open(archive, "r:") as stream:
    for item in stream:
        name = PurePosixPath(item.name)
        if name.is_absolute() or not name.parts or any(part in ("", ".", "..") for part in name.parts):
            raise ValueError("unsafe corpus archive path")
        target = realistic.joinpath(*name.parts)
        if item.isdir():
            target.mkdir(mode=0o700, parents=True, exist_ok=True)
        elif item.isfile():
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            with stream.extractfile(item) as source, target.open("xb") as destination:
                while chunk := source.read(1024 * 1024):
                    destination.write(chunk)
        else:
            raise ValueError("corpus archive contains a non-regular entry")

corpora = [(realistic, "realistic", None)]
for symbols in symbol_counts:
    minified = work / f"minified-{symbols}"
    minified.mkdir(mode=0o700)
    source = "".join(
        f"fn symbol_{index:05}() {{ consume_distinct_word_{index:05}(); }}"
        for index in range(symbols)
    ).encode()
    target = minified / "src/minified.rs"
    target.parent.mkdir(mode=0o700)
    with target.open("xb") as stream:
        stream.write(source)
    corpora.append((minified, f"minified-{symbols}", symbols))

for root, name, symbols in corpora:
    digest = hashlib.sha256()
    file_count = 0
    total_bytes = 0
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        relative = path.relative_to(root).as_posix().encode()
        content = path.read_bytes()
        digest.update(relative + b"\0" + len(content).to_bytes(8, "big") + content)
        file_count += 1
        total_bytes += len(content)
    record = {"corpus": name, "files": file_count, "bytes": total_bytes,
              "sha256": digest.hexdigest()}
    if symbols is not None:
        record["symbols"] = symbols
    print(json.dumps(record, sort_keys=True))
PYTHON

BASELINE_SHA="$(shasum -a 256 "${BASELINE}" | awk '{print $1}')"
CANDIDATE_SHA="$(shasum -a 256 "${CANDIDATE}" | awk '{print $1}')"
if [[ "$("${BASELINE}" version)" != "$("${CANDIDATE}" version)" ]]; then
  echo "measure-symbol-ownership.sh: binary versions differ" >&2
  exit 1
fi
printf 'corpus_commit=%s\nbaseline_sha256=%s\ncandidate_sha256=%s\nruns_per_binary=%s\n' \
  "${REVISION}" "${BASELINE_SHA}" "${CANDIDATE_SHA}" "${RUNS}"

index_once() {
  local binary="$1" corpus="$2" index_base="$3"
  mkdir -m 700 "${index_base}"
  "${binary}" index -C "${corpus}" --index-dir "${index_base}" >/dev/null
}

compare_indexes() {
  python3 -I - "$1" "$2" <<'PYTHON'
import hashlib
from pathlib import Path
import stat
import sys

def snapshot(root):
    base = Path(root)
    result = {}
    metadata = {}
    total_bytes = 0
    for path in base.rglob("*"):
        info = path.lstat()
        if stat.S_ISLNK(info.st_mode):
            raise ValueError("index contains a symlink")
        if stat.S_ISREG(info.st_mode):
            total_bytes += info.st_size
            relative = path.relative_to(base).as_posix()
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            if path.name == "meta":
                metadata[relative] = (info.st_size, digest)
            else:
                result[relative] = digest
        elif not stat.S_ISDIR(info.st_mode):
            raise ValueError("index contains a non-regular file")
    return result, metadata, total_bytes

left, left_metadata, left_bytes = snapshot(sys.argv[1])
right, right_metadata, right_bytes = snapshot(sys.argv[2])
if left != right:
    paths = sorted(set(left) | set(right))
    changed = [path for path in paths if left.get(path) != right.get(path)]
    raise SystemExit("release index outputs differ: " + ", ".join(changed[:12]))
if (left_metadata.keys() != right_metadata.keys()
        or any(left_metadata[path][0] != right_metadata[path][0] for path in left_metadata)
        or left_bytes != right_bytes):
    raise SystemExit("release index metadata structure or size differs")
print(f"verified_index_files={len(left)} metadata_records={len(left_metadata)} output_bytes={left_bytes}")
PYTHON
}

write_runner() {
  local runner="$1"
  ( set -o noclobber
    cat > "${runner}" <<'BASH'
#!/usr/bin/env bash
set -euo pipefail
binary="$1"
corpus="$2"
work="$3"
index_base="$(mktemp -d "${work}/index.XXXXXXXX")"
cleanup() {
  local status=$?
  trap - EXIT
  case "${index_base}" in
    "${work}/index."*)
      if [[ -d "${index_base}" && ! -L "${index_base}" ]]; then
        rm -rf -- "${index_base}" || status=1
      else
        echo "benchmark runner: refusing to remove changed index path" >&2
        status=1
      fi
      ;;
    *)
      echo "benchmark runner: refusing to remove unexpected index path" >&2
      status=1
      ;;
  esac
  exit "${status}"
}
trap cleanup EXIT
"${binary}" index -C "${corpus}" --index-dir "${index_base}" >/dev/null
BASH
  )
}

shell_command() {
  python3 -I -c 'import shlex,sys; print(shlex.join(sys.argv[1:]))' "$@"
}

measure_once() {
  local binary_name="$1" corpus_name="$2" cmd="$3" run="$4" json elapsed rss load
  json="${WORK}/sample-${corpus_name}-${binary_name}-${run}.json"
  hyperfine -N --metrics time_wall_clock:ms,memory_peak_resident:MiB --warmup 0 --runs 1 \
    --export-json "${json}" "${cmd}" >/dev/null
  elapsed="$(jq -er '.results[0].measurements[0].time_wall_clock.value' "${json}")"
  rss="$(jq -er '.results[0].measurements[0].memory_peak_resident.value' "${json}")"
  load="$(python3 -I -c 'import os; print(",".join(f"{value:.6f}" for value in os.getloadavg()))')"
  printf '%s\t%s\t%s\t%s\t%s\n' "${binary_name}" "${run}" "${elapsed}" "${rss}" "${load}" \
    >> "${WORK}/observations-${corpus_name}.tsv"
}

summarize() {
  python3 -I - "$1" "$2" <<'PYTHON'
import json
from pathlib import Path
import statistics
import sys

name, path = sys.argv[1:]
samples = {"baseline": [], "candidate": []}
for line in Path(path).read_text().splitlines():
    binary, run, seconds, rss, load = line.split("\t")
    samples[binary].append((int(run), float(seconds), float(rss),
                           tuple(float(value) for value in load.split(","))))
for values in samples.values():
    values.sort()
baseline = [seconds for _, seconds, _, _ in samples["baseline"]]
candidate = [seconds for _, seconds, _, _ in samples["candidate"]]
paired = [1 - candidate[index] / baseline[index] for index in range(min(len(baseline), len(candidate)))]
if not baseline or len(baseline) != len(candidate):
    raise SystemExit("incomplete paired benchmark samples")
baseline_rss = [rss for _, _, rss, _ in samples["baseline"]]
candidate_rss = [rss for _, _, rss, _ in samples["candidate"]]
loads = [load for group in samples.values() for _, _, _, load in group]
median_load = [round(statistics.median(sample[index] for sample in loads), 3) for index in range(3)]
peak_one_minute_load = round(max(sample[0] for sample in loads), 3)
print(json.dumps({"corpus": name,
                  "baseline_median_ms": round(statistics.median(baseline) * 1000, 3),
                  "candidate_median_ms": round(statistics.median(candidate) * 1000, 3),
                  "median_paired_reduction_pct": round(statistics.median(paired) * 100, 2),
                  "baseline_median_peak_rss_mib": round(statistics.median(baseline_rss) / 1024**2, 3),
                  "candidate_median_peak_rss_mib": round(statistics.median(candidate_rss) / 1024**2, 3),
                  "baseline_min_ms": round(min(baseline) * 1000, 3),
                  "candidate_min_ms": round(min(candidate) * 1000, 3),
                  "baseline_max_ms": round(max(baseline) * 1000, 3),
                  "candidate_max_ms": round(max(candidate) * 1000, 3),
                  "load_average_median_1_5_15m": median_load,
                  "load_average_peak_1m": peak_one_minute_load,
                  "paired_samples": len(paired)}, sort_keys=True))
PYTHON
}

write_runner "${WORK}/run-index.sh"
corpus_names=(realistic)
for symbol_count in "${STRESS_SYMBOL_COUNTS[@]}"; do
  corpus_names+=("minified-${symbol_count}")
done
for corpus_name in "${corpus_names[@]}"; do
  corpus="${WORK}/${corpus_name}"
  mkdir -m 700 "${WORK}/verify-${corpus_name}"
  index_once "${BASELINE}" "${corpus}" "${WORK}/verify-${corpus_name}/baseline"
  index_once "${CANDIDATE}" "${corpus}" "${WORK}/verify-${corpus_name}/candidate"
  compare_indexes "${WORK}/verify-${corpus_name}/baseline" "${WORK}/verify-${corpus_name}/candidate"

  observations="${WORK}/observations-${corpus_name}.tsv"
  ( set -o noclobber; : > "${observations}" )
  baseline_command="$(shell_command bash "${WORK}/run-index.sh" "${BASELINE}" "${corpus}" "${WORK}")"
  candidate_command="$(shell_command bash "${WORK}/run-index.sh" "${CANDIDATE}" "${corpus}" "${WORK}")"
  for warmup in 1 2; do
    if (( (warmup + ${#corpus_name}) % 2 )); then
      bash "${WORK}/run-index.sh" "${BASELINE}" "${corpus}" "${WORK}"
      bash "${WORK}/run-index.sh" "${CANDIDATE}" "${corpus}" "${WORK}"
    else
      bash "${WORK}/run-index.sh" "${CANDIDATE}" "${corpus}" "${WORK}"
      bash "${WORK}/run-index.sh" "${BASELINE}" "${corpus}" "${WORK}"
    fi
  done
  for ((run = 1; run <= RUNS; run++)); do
    if (( run % 2 )); then
      measure_once baseline "${corpus_name}" "${baseline_command}" "${run}"
      measure_once candidate "${corpus_name}" "${candidate_command}" "${run}"
    else
      measure_once candidate "${corpus_name}" "${candidate_command}" "${run}"
      measure_once baseline "${corpus_name}" "${baseline_command}" "${run}"
    fi
  done
  summarize "${corpus_name}" "${observations}"
done
