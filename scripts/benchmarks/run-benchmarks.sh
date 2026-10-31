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

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly WORK_BASE="${REPO_ROOT}/.cache/benchmarks"
readonly BINARY="${REPO_ROOT}/target/release/xcalygraph"
readonly PRIVATE_LIST="${REPO_ROOT}/scripts/benchmarks/questions/private/corpora.tsv"
readonly COPY_LIMIT=20000

upstream=0
if [[ "${1:-}" == "--upstream" ]]; then
  upstream=1
  shift
fi
tools=(hyperfine jq rsync python3)
(( upstream )) && tools+=(graphify)
for tool in "${tools[@]}"; do
  command -v "${tool}" >/dev/null || { echo "run-benchmarks.sh: ${tool} missing" >&2; exit 1; }
done

cd "${REPO_ROOT}"
cargo build --release --locked -q
mkdir -p "${WORK_BASE}"
WORK="$(mktemp -d "${WORK_BASE}/run.XXXXXXXX")"
readonly WORK
exec > >(tee "${WORK}/results.txt")
readonly BENCHMARK_HOME="${WORK}/home"
mkdir -p "${BENCHMARK_HOME}"
printf 'binary_sha256\t%s\n' "$(python3 -I -c 'import hashlib, pathlib, sys; print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())' "${BINARY}")" > "${WORK}/identity.tsv"

corpora=("xcalygraph	${REPO_ROOT}")
[[ -d research/upstream/graphify ]] && corpora+=("graphify	${REPO_ROOT}/research/upstream/graphify")
if [[ -d research/corpora ]]; then
  for dir in research/corpora/*/; do
    corpora+=("$(basename "${dir}")	${REPO_ROOT}/${dir%/}")
  done
fi
if [[ -f "${PRIVATE_LIST}" ]]; then
  while IFS=$'\t' read -r name path; do
    [[ -z "${name}" || "${name}" == \#* ]] && continue
    corpora+=("${name}	${path}")
  done < "${PRIVATE_LIST}"
fi

wanted=("$@")
selected() {
  (( ${#wanted[@]} == 0 )) && return 0
  local name
  for name in "${wanted[@]}"; do [[ "${name}" == "$1" ]] && return 0; done
  return 1
}

# Mean wall time in milliseconds of a command, from hyperfine's JSON export.
mean_ms() {
  local json
  json="$(mktemp "${WORK}/timing-${name//\//-}.json.XXXXXXXX")" || return 1
  hyperfine -N --warmup 3 --runs 15 --export-json "${json}" "$@" >"${json}.log" 2>&1 || return 1
  jq -er '.results[0].summary.time_wall_clock.mean * 10000 | round / 10' "${json}"
}

# Sum of sizes of index files whose inode changed since the snapshot (atomic rename = rewrite).
bytes_rewritten() {
  local index="$1" snapshot="$2"
  find "${index}" -type f -exec stat -f '%i %z %N' {} + | sort > "${snapshot}.after"
  comm -13 "${snapshot}" "${snapshot}.after" | awk '{ s += $2 } END { print s + 0 }'
}

# Graphify full build and single-file update on the copy, appended to the result line as
# "up_build_ms up_rss_mb up_out_kb up_change_ms"; runs before the copy gets xcalygraph changes.
upstream_columns() {
  local copy="$1" target="$2" edit="$3" build change
  if ! build="$( { cd "${copy}" && /usr/bin/time -l env HOME="${BENCHMARK_HOME}" PYTHONHASHSEED=0 PYTHONSAFEPATH=1 PYTHONDONTWRITEBYTECODE=1 graphify update . >/dev/null ; } 2>&1 )"; then
    printf '%s\n' "${build}" > "${copy}.upstream-full.time"
    echo "run-benchmarks.sh: upstream extraction failed; inspect ${copy}.upstream-full.time" >&2
    return 1
  fi
  printf '%s\n' "${build}" > "${copy}.upstream-full.time"
  printf ' %12s %9s %9s' \
    "$(awk '/ real / { printf "%.0f", $1 * 1000 }' <<<"${build}")" \
    "$(awk '/maximum resident set size/ { printf "%.0f", $1 / 1048576 }' <<<"${build}")" \
    "$(du -sk "${copy}/graphify-out" | awk '{ print $1 }')"
  printf '%s' "${edit}" >> "${target}"
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
for entry in "${corpora[@]}"; do
  name="${entry%%	*}"
  path="${entry#*	}"
  selected "${name}" || continue
  [[ -d "${path}" ]] || { echo "run-benchmarks.sh: ${name}: ${path} missing" >&2; continue; }
  path="$(bash "${REPO_ROOT}/scripts/utils/prepare-corpus.sh" "${path}" "${REPO_ROOT}/.cache/corpus-snapshots")"
  printf '%s\t%s\n' "${name}" "${path}" >> "${WORK}/identity.tsv"
  index="${WORK}/index-${name}"
  rm -rf "${index}"
  build="$( { /usr/bin/time -l env HOME="${BENCHMARK_HOME}" "${BINARY}" index -C "${path}" --index-dir "${index}" >"${WORK}/out" ; } 2>&1 )"
  printf '%s\n' "${build}" > "${WORK}/${name//\//-}.full.time"
  files="$(awk '{ print $2 }' "${WORK}/out")"
  build_ms="$(awk '/ real / { printf "%.0f", $1 * 1000 }' <<<"${build}")"
  rss_mb="$(awk '/maximum resident set size/ { printf "%.0f", $1 / 1048576 }' <<<"${build}")"
  fresh_ms="$(mean_ms "${BINARY} index -C ${path} --index-dir ${index}")"
  index_kb="$(du -sk "${index}" | awk '{ print $1 }')"
  change_ms="-"
  write_kb="-"
  upstream_line=""
  if (( files <= COPY_LIMIT )); then
    copy="${WORK}/copy-${name}"
    copy_index="${WORK}/index-copy-${name}"
    rm -rf "${copy}" "${copy_index}"
    mkdir -p "${copy}"
    rsync -a "${path}/" "${copy}/"
    "${BINARY}" index -C "${copy}" --index-dir "${copy_index}" >/dev/null
    # A tracked source file, so ignore rules cannot hide the change.
    tracked="$(git -C "${path}" ls-files -- '*.go' '*.rs' '*.py' '*.ts' '*.java' '*.kt' '*.cs' '*.c' '*.cc' '*.cpp' '*.m' '*.swift' 2>/dev/null | awk 'NR == 1')"
    if [[ -n "${tracked}" ]]; then
      target="${copy}/${tracked}"
    else
      target="$(find "${copy}" -type f \( -name '*.go' -o -name '*.rs' -o -name '*.py' -o -name '*.ts' -o -name '*.java' -o -name '*.kt' -o -name '*.cs' -o -name '*.c' -o -name '*.cc' -o -name '*.cpp' -o -name '*.m' -o -name '*.swift' \) -print -quit)"
    fi
    if [[ -n "${target}" ]]; then
      # A comment line valid in the file's language, so both tools see a parseable change.
      edit=$'\n// bench\n'
      [[ "${target}" == *.py ]] && edit=$'\n# bench\n'
      (( upstream )) && upstream_line="$(upstream_columns "${copy}" "${target}" "${edit}")"
      sleep 2
      find "${copy_index}" -type f -exec stat -f '%i %z %N' {} + | sort > "${WORK}/snapshot"
      change_ms="$(mean_ms --prepare "printf '%s' '${edit}' >> '${target}'" "${BINARY} index -C ${copy} --index-dir ${copy_index}")"
      printf '%s' "${edit}" >> "${target}"
      find "${copy_index}" -type f -exec stat -f '%i %z %N' {} + | sort > "${WORK}/snapshot"
      printf '%s' "${edit}" >> "${target}"
      "${BINARY}" index -C "${copy}" --index-dir "${copy_index}" >/dev/null
      write_kb="$(( $(bytes_rewritten "${copy_index}" "${WORK}/snapshot") / 1024 ))"
    fi
    rm -rf "${copy}" "${copy_index}"
  fi
  printf '%-14s %8s %9s %9s %10s %10s %9s %9s%s\n' \
    "${name}" "${files}" "${build_ms}" "${rss_mb}" "${fresh_ms}" "${change_ms}" "${index_kb}" "${write_kb}" "${upstream_line}"
done
echo "evidence: ${WORK} (identity.tsv and measurement artifacts)"
