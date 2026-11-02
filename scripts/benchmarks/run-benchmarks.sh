#!/usr/bin/env bash
# Measure index build, freshness, single-file refresh and footprint on the reference corpora.
# Usage: scripts/benchmarks/run-benchmarks.sh [--upstream] [CORPUS...]
#   Corpora: this repository, the upstream clone (research/upstream/graphify), pinned public
#   corpora under research/corpora/ and private corpora listed as "name<TAB>path" in the
#   gitignored scripts/benchmarks/questions/private/corpora.tsv. CORPUS names restrict the run.
#   Corpora are read-only: the single-file refresh runs on a copy below .cache/.
#   --upstream adds the Graphify baseline on the same copy: graphify update . for the full build,
#   peak RSS and graphify-out size, then again after the same single-file change.
# Requires: hyperfine, jq, rsync, python3; graphify for --upstream; the release build is made here.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
readonly REPO_ROOT
readonly WORK_BASE="${REPO_ROOT}/.cache/benchmarks"
readonly PRIVATE_LIST="${REPO_ROOT}/scripts/benchmarks/questions/private/corpora.tsv"
readonly COPY_LIMIT=20000

hyperfine_command() {
  python3 -I -c 'import shlex,sys; print(shlex.join(sys.argv[1:]))' "$@"
}

hyperfine_append_command() {
  local command target
  command="$(hyperfine_command printf '%s' "$1")" || return
  target="$(hyperfine_command "$2")" || return
  printf '%s >> %s\n' "${command}" "${target}"
}

directory_kb() {
  local output value
  output="$(du -sk -- "$1")" || return 1
  value="${output%%[[:space:]]*}"
  [[ "${value}" =~ ^[0-9]+$ ]] || { echo "run-benchmarks.sh: invalid du output" >&2; return 1; }
  printf '%s\n' "${value}"
}

# Snapshot regular-file identities, then sum the logical size of new or changed identities.
measure_index_files() {
  python3 -I - "$@" <<'PYTHON'
import json
import os
import stat
import sys

action, directory, snapshot = sys.argv[1:]
if action not in ("snapshot", "bytes"):
    raise ValueError("invalid index measurement action")
root = os.path.abspath(directory)
if not stat.S_ISDIR(os.lstat(root).st_mode):
    raise ValueError("index root is not a directory")
previous = None
if action == "bytes":
    with open(snapshot, encoding="utf-8") as stream:
        previous = json.load(stream)
    if (not isinstance(previous, dict) or set(previous) != {"root", "files"} or previous["root"] != root
            or not isinstance(previous["files"], dict)):
        raise ValueError("invalid index snapshot")
    for path, identity in previous["files"].items():
        if (not isinstance(path, str) or not path or os.path.isabs(path) or ".." in path.split(os.sep)
                or not isinstance(identity, list) or len(identity) != 3
                or any(type(value) is not int or value < 0 for value in identity)):
            raise ValueError("invalid index snapshot record")

def walk_error(error):
    raise error

files = {}
for parent, _, names in os.walk(root, onerror=walk_error, followlinks=False):
    for name in names:
        path = os.path.join(parent, name)
        info = os.lstat(path)
        if stat.S_ISREG(info.st_mode):
            files[os.path.relpath(path, root)] = [info.st_dev, info.st_ino, info.st_size]
output = snapshot if action == "snapshot" else snapshot + ".after"
with open(output, "x", encoding="utf-8") as stream:
    json.dump({"root": root, "files": files}, stream, ensure_ascii=True, sort_keys=True)
if previous is not None:
    print(sum(identity[2] for path, identity in files.items() if previous["files"].get(path) != identity))
PYTHON
}

# Preserve every candidate pathname exactly and fail if enumeration fails.
enumerate_source_paths() {
  local copy="$1" record="$2"
  ( set -o noclobber
    find "${copy}" -type f \( -name '*.go' -o -name '*.rs' -o -name '*.py' -o -name '*.ts' -o -name '*.java' -o -name '*.kt' -o -name '*.cs' -o -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.m' -o -name '*.swift' \) -print0 > "${record}"
  )
}

# Return success only when NAME resolves to this file and its extracted symbols are indexed.
source_has_symbols() {
  python3 -I - "$@" <<'PYTHON'
import json
import subprocess
import sys

binary, root, index, path = sys.argv[1:]
try:
    path.encode("utf-8")
except UnicodeEncodeError:
    raise SystemExit(1)
try:
    result = subprocess.run(
        [binary, "show", "--json", "--read-only", "-b", "65536", "-d", "0", "-C", root,
         "--index-dir", index, "--", path],
        capture_output=True,
    )
except OSError as error:
    print(f"run-benchmarks.sh: could not run source query: {error}", file=sys.stderr)
    raise SystemExit(2)
if result.returncode in (3, 4):
    raise SystemExit(1)
if result.returncode != 0:
    sys.stderr.buffer.write(result.stderr)
    print("run-benchmarks.sh: could not verify selected source", file=sys.stderr)
    raise SystemExit(2)
try:
    records = [json.loads(line) for line in result.stdout.splitlines()]
except (UnicodeDecodeError, json.JSONDecodeError) as error:
    print(f"run-benchmarks.sh: invalid source query output: {error}", file=sys.stderr)
    raise SystemExit(2)
if not all(isinstance(record, dict) for record in records):
    print("run-benchmarks.sh: invalid source query record", file=sys.stderr)
    raise SystemExit(2)
if (not records or records[0].get("t") != "file" or records[0].get("path") != path
        or not any(record.get("t") == "sym" and record.get("path") == path for record in records[1:])):
    raise SystemExit(1)
PYTHON
}

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
  return 0
fi

original_args=("$@")
CALLER_DIRECTORY="$(pwd -P && printf '.')"
CALLER_DIRECTORY="${CALLER_DIRECTORY%.}"
CALLER_DIRECTORY="${CALLER_DIRECTORY%$'\n'}"
readonly CALLER_DIRECTORY
upstream=0
if [[ "${1:-}" == "--upstream" ]]; then
  upstream=1
  shift
fi
tools=(hyperfine jq rsync python3)
(( upstream )) && tools+=(graphify)

cd "${REPO_ROOT}"
corpora=("xcalygraph"$'\t'"${REPO_ROOT}")
if [[ -d research/upstream/graphify ]]; then
  corpora+=("graphify"$'\t'"${REPO_ROOT}/research/upstream/graphify")
fi
if [[ -d research/corpora ]]; then
  for dir in research/corpora/*/; do
    [[ -d "${dir}" ]] || continue
    name="${dir%/}"
    name="${name##*/}"
    [[ "${name}" != *[[:cntrl:]]* ]] || { echo "run-benchmarks.sh: invalid corpus name" >&2; exit 2; }
    corpora+=("${name}"$'\t'"${REPO_ROOT}/${dir%/}")
  done
fi
if [[ -f "${PRIVATE_LIST}" ]]; then
  private_records="$(bash "${REPO_ROOT}/scripts/utils/read-private-corpus.sh" list "${PRIVATE_LIST}" && printf '.')"
  private_records="${private_records%.}"
  while IFS=$'\t' read -r name path; do
    [[ -z "${name}" || "${name}" == \#* ]] && continue
    if [[ "${path}" != /* ]]; then path="${CALLER_DIRECTORY}/${path}"; fi
    corpora+=("${name}"$'\t'"${path}")
  done <<< "${private_records}"
fi
for (( corpus_position = 0; corpus_position < ${#corpora[@]}; corpus_position++ )); do
  name="${corpora[corpus_position]%%$'\t'*}"
  for (( previous_position = 0; previous_position < corpus_position; previous_position++ )); do
    if [[ "${corpora[previous_position]%%$'\t'*}" == "${name}" ]]; then
      echo "run-benchmarks.sh: duplicate corpus: ${name}" >&2
      exit 2
    fi
  done
done
wanted=("$@")
selected() {
  (( ${#wanted[@]} == 0 )) && return 0
  local name
  for name in "${wanted[@]}"; do [[ "${name}" == "$1" ]] && return 0; done
  return 1
}
for requested in ${wanted[@]+"${wanted[@]}"}; do
  found=0
  for entry in "${corpora[@]}"; do
    name="${entry%%$'\t'*}"
    if [[ "${name}" == "${requested}" ]]; then found=1; break; fi
  done
  (( found )) || { echo "run-benchmarks.sh: unknown corpus: ${requested}" >&2; exit 2; }
done
selected_count=0
available_corpora=()
for entry in "${corpora[@]}"; do
  name="${entry%%$'\t'*}"
  path="${entry#*$'\t'}"
  selected "${name}" || continue
  if [[ ! -d "${path}" ]]; then
    echo "run-benchmarks.sh: missing corpus ${name}: ${path}" >&2
    (( ${#wanted[@]} == 0 )) && continue
    exit 1
  fi
  selected_count=$((selected_count + 1))
  available_corpora+=("${entry}")
done
(( selected_count > 0 )) || { echo "run-benchmarks.sh: no corpora to measure" >&2; exit 1; }

for tool in "${tools[@]}"; do
  command -v "${tool}" >/dev/null || { echo "run-benchmarks.sh: ${tool} missing" >&2; exit 1; }
done

if [[ -L "${REPO_ROOT}/.cache" || -L "${WORK_BASE}" ]]; then
  echo "run-benchmarks.sh: benchmark cache must not be a symlink" >&2
  exit 1
fi
selection_sha256="$(printf '%s\0' "${available_corpora[@]}" | python3 -I -c 'import hashlib,sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())')"
if ! bash "${REPO_ROOT}/scripts/utils/verification-lock.sh"; then
  exec bash "${REPO_ROOT}/scripts/utils/guard-benchmark.sh" \
    bash -c 'expected="$1"; cd -- "$2"; shift 2; XCALYGRAPH_BENCHMARK_SELECTION_SHA256="$expected" exec "$@"' \
    benchmark-reentry "${selection_sha256}" "${CALLER_DIRECTORY}" \
    bash "${REPO_ROOT}/scripts/benchmarks/run-benchmarks.sh" ${original_args[@]+"${original_args[@]}"}
fi
if [[ -n "${XCALYGRAPH_BENCHMARK_SELECTION_SHA256:-}" && "${XCALYGRAPH_BENCHMARK_SELECTION_SHA256}" != "${selection_sha256}" ]]; then
  echo "run-benchmarks.sh: corpus selection changed before the guard" >&2
  exit 1
fi
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
BINARY="$(cargo build --release --locked --bin xcalygraph --message-format=json -q \
  | bash "${REPO_ROOT}/scripts/utils/cargo-artifact.sh")"
readonly BINARY
mkdir -p "${WORK_BASE}"
WORK="$(mktemp -d "${WORK_BASE}/run.XXXXXXXX")"
readonly WORK
source "${REPO_ROOT}/scripts/benchmarks/output-writer.sh"
start_output_writer "${WORK}/results.txt"
readonly BENCHMARK_HOME="${WORK}/home"
mkdir -p "${BENCHMARK_HOME}"
printf 'binary_sha256\t%s\n' "$(python3 -I -c 'import hashlib, pathlib, sys; print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())' "${BINARY}")" > "${WORK}/identity.tsv"

# Mean wall time in milliseconds of a command, from hyperfine's JSON export.
mean_ms() {
  local json
  json="$(mktemp "${WORK}/timing-${corpus_number}.json.XXXXXXXX")" || return 1
  hyperfine -N --warmup 3 --runs 15 --export-json "${json}" "$@" >"${json}.log" 2>&1 || return 1
  jq -er '.results[0].summary.time_wall_clock.mean * 10000 | round / 10' "${json}"
}

# Measure Graphify's full build before the verified edit; return its three result columns.
upstream_build_columns() {
  local copy="$1" build upstream_kb
  if ! build="$( { cd "${copy}" && /usr/bin/time -l env HOME="${BENCHMARK_HOME}" PYTHONHASHSEED=0 PYTHONSAFEPATH=1 PYTHONDONTWRITEBYTECODE=1 graphify update . >/dev/null ; } 2>&1 )"; then
    printf '%s\n' "${build}" > "${copy}.upstream-full.time"
    echo "run-benchmarks.sh: upstream extraction failed; inspect ${copy}.upstream-full.time" >&2
    return 1
  fi
  printf '%s\n' "${build}" > "${copy}.upstream-full.time"
  upstream_kb="$(directory_kb "${copy}/graphify-out")" || return 1
  printf ' %12s %9s %9s' \
    "$(awk '/ real / { printf "%.0f", $1 * 1000 }' <<<"${build}")" \
    "$(awk '/maximum resident set size/ { printf "%.0f", $1 / 1048576 }' <<<"${build}")" \
    "${upstream_kb}"
}

# Measure Graphify's incremental refresh after the xcalygraph probe proves the same edit.
upstream_change_column() {
  local copy="$1" change
  if ! change="$( { cd "${copy}" && /usr/bin/time -l env HOME="${BENCHMARK_HOME}" PYTHONHASHSEED=0 PYTHONSAFEPATH=1 PYTHONDONTWRITEBYTECODE=1 graphify update . >/dev/null ; } 2>&1 )"; then
    printf '%s\n' "${change}" > "${copy}.upstream-change.time"
    echo "run-benchmarks.sh: upstream refresh failed; inspect ${copy}.upstream-change.time" >&2
    return 1
  fi
  printf '%s\n' "${change}" > "${copy}.upstream-change.time"
  printf ' %12s' "$(awk '/ real / { printf "%.0f", $1 * 1000 }' <<<"${change}")"
  rm -rf "${copy}/graphify-out"
}

header="$(printf '%-14s %8s %9s %9s %10s %10s %9s %9s' corpus files build_ms rss_mb fresh_ms change_ms index_kb write_kb)"
(( upstream )) && header+="$(printf ' %12s %9s %9s %12s' up_build_ms up_rss_mb up_out_kb up_change_ms)"
echo "${header}"
corpus_number=0
for entry in "${available_corpora[@]}"; do
  name="${entry%%	*}"
  path="${entry#*	}"
  corpus_number=$((corpus_number + 1))
  [[ -d "${path}" ]] || { echo "run-benchmarks.sh: ${name}: ${path} missing" >&2; exit 1; }
  input_root="${path}"
  path="$(bash "${REPO_ROOT}/scripts/utils/prepare-corpus.sh" "${path}" "${REPO_ROOT}/.cache/corpus-snapshots")"
  python3 -I - "${corpus_number}" "${name}" "${input_root}" "${path}" >> "${WORK}/identity.tsv" <<'PYTHON'
import json
import sys
print("corpus\t" + sys.argv[1] + "\t" + "\t".join(json.dumps(value, ensure_ascii=True) for value in sys.argv[2:]))
PYTHON
  index="${WORK}/index-${corpus_number}"
  [[ ! -e "${index}" && ! -L "${index}" ]] || { echo "run-benchmarks.sh: index path already exists" >&2; exit 1; }
  build="$( { /usr/bin/time -l env HOME="${BENCHMARK_HOME}" "${BINARY}" index -C "${path}" --index-dir "${index}" >"${WORK}/out" ; } 2>&1 )"
  printf '%s\n' "${build}" > "${WORK}/corpus-${corpus_number}.full.time"
  files="$(awk '{ print $2 }' "${WORK}/out")"
  build_ms="$(awk '/ real / { printf "%.0f", $1 * 1000 }' <<<"${build}")"
  rss_mb="$(awk '/maximum resident set size/ { printf "%.0f", $1 / 1048576 }' <<<"${build}")"
  fresh_command="$(hyperfine_command "${BINARY}" index -C "${path}" --index-dir "${index}")"
  fresh_ms="$(mean_ms "${fresh_command}")"
  index_kb="$(directory_kb "${index}")"
  change_ms="-"
  write_kb="-"
  upstream_line=""
  if (( files <= COPY_LIMIT )); then
    copy="${WORK}/copy-${corpus_number}"
    copy_index="${WORK}/index-copy-${corpus_number}"
    [[ ! -e "${copy}" && ! -L "${copy}" && ! -e "${copy_index}" && ! -L "${copy_index}" ]] || {
      echo "run-benchmarks.sh: copy path already exists" >&2; exit 1
    }
    mkdir "${copy}"
    rsync -a "${path}/" "${copy}/"
    "${BINARY}" index -C "${copy}" --index-dir "${copy_index}" >/dev/null
    source_record="${WORK}/source-${corpus_number}.path"
    candidate_record="${WORK}/source-${corpus_number}.candidates"
    enumerate_source_paths "${copy}" "${candidate_record}"
    target=""
    while IFS= read -r -d '' candidate; do
      case "${candidate}" in
        "${copy}/"*) relative="${candidate#"${copy}/"}" ;;
        *) echo "run-benchmarks.sh: source candidate escaped its copy" >&2; exit 1 ;;
      esac
      if source_has_symbols "${BINARY}" "${copy}" "${copy_index}" "${relative}"; then
        target="${candidate}"
        break
      else
        source_status=$?
        (( source_status == 1 )) || exit "${source_status}"
      fi
    done < "${candidate_record}"
    if [[ -n "${target}" ]]; then
      ( set -o noclobber; printf '%s\0' "${target}" > "${source_record}" )
    else
      ( set -o noclobber; : > "${source_record}" )
    fi
    if [[ -n "${target}" ]]; then
      # A trailing newline is syntax-neutral across the supported source languages.
      edit=$'\n'
      if (( upstream )); then
        upstream_line="$(upstream_build_columns "${copy}")" || exit 1
      fi
      printf '%s' "${edit}" >> "${target}"
      probe_output="$("${BINARY}" index -C "${copy}" --index-dir "${copy_index}")"
      if [[ ! "${probe_output}" =~ ^files[[:space:]][0-9]+[[:space:]]changed[[:space:]]1[[:space:]]deleted[[:space:]]0$ ]]; then
        echo "run-benchmarks.sh: selected source did not trigger exactly one index change" >&2
        exit 1
      fi
      ( set -o noclobber; printf '%s\n' "${probe_output}" > "${WORK}/probe-${corpus_number}.txt" )
      relative="${target#"${copy}/"}"
      source_has_symbols "${BINARY}" "${copy}" "${copy_index}" "${relative}" || {
        echo "run-benchmarks.sh: selected source lost its extracted symbols after the probe" >&2
        exit 1
      }
      if (( upstream )); then upstream_line+="$(upstream_change_column "${copy}")" || exit 1; fi
      sleep 2
      prepare_command="$(hyperfine_append_command "${edit}" "${target}")"
      change_command="$(hyperfine_command "${BINARY}" index -C "${copy}" --index-dir "${copy_index}")"
      change_ms="$(mean_ms --prepare "${prepare_command}" "${change_command}")"
      printf '%s' "${edit}" >> "${target}"
      snapshot="${WORK}/snapshot-${corpus_number}.json"
      measure_index_files snapshot "${copy_index}" "${snapshot}"
      printf '%s' "${edit}" >> "${target}"
      "${BINARY}" index -C "${copy}" --index-dir "${copy_index}" >/dev/null
      source_has_symbols "${BINARY}" "${copy}" "${copy_index}" "${relative}" || {
        echo "run-benchmarks.sh: selected source lost its extracted symbols after timing" >&2
        exit 1
      }
      rewritten_bytes="$(measure_index_files bytes "${copy_index}" "${snapshot}")"
      write_kb="$((rewritten_bytes / 1024))"
    else
      echo "run-benchmarks.sh: no indexed source with extracted symbols; refresh not measured" >&2
      (( upstream )) && upstream_line="$(printf ' %12s %9s %9s %12s' - - - -)"
    fi
    rm -rf "${copy}" "${copy_index}"
  fi
  printf '%-14s %8s %9s %9s %10s %10s %9s %9s%s\n' \
    "${name}" "${files}" "${build_ms}" "${rss_mb}" "${fresh_ms}" "${change_ms}" "${index_kb}" "${write_kb}" "${upstream_line}"
done
echo "evidence: ${WORK} (identity.tsv and measurement artifacts)"
finish_output_writer
( set -o noclobber; printf 'xcalygraph-cache-v1\n' > "${WORK}/.xcalygraph-disposable" )
