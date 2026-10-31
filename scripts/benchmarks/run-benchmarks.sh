#!/usr/bin/env bash
# Measure index build, freshness, single-file refresh and footprint on the reference corpora.
# Usage: scripts/benchmarks/run-benchmarks.sh [--upstream] [CORPUS...]
#   Corpora: this repository, the upstream clone (research/upstream/graphify), pinned public
#   corpora under research/corpora/ and private corpora listed as "name<TAB>path" in the
#   gitignored scripts/benchmarks/questions/private/corpora.tsv. CORPUS names restrict the run.
#   Corpora are read-only: the single-file refresh runs on a copy below .cache/.
#   --upstream adds the Graphify baseline on the same copy: graphify update . for the full build,
#   peak RSS and graphify-out size, then again after the same single-file change.
# Requires: hyperfine, jq, rsync; graphify for --upstream; the release build is made here.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly WORK="${REPO_ROOT}/.cache/benchmarks"
readonly BINARY="${REPO_ROOT}/target/release/xcalygraph"
readonly PRIVATE_LIST="${REPO_ROOT}/scripts/benchmarks/questions/private/corpora.tsv"
readonly COPY_LIMIT=20000

upstream=0
if [[ "${1:-}" == "--upstream" ]]; then
  upstream=1
  shift
fi
tools=(hyperfine jq rsync)
(( upstream )) && tools+=(graphify)
for tool in "${tools[@]}"; do
  command -v "${tool}" >/dev/null || { echo "run-benchmarks.sh: ${tool} missing" >&2; exit 1; }
done

cd "${REPO_ROOT}"
cargo build --release --locked -q
mkdir -p "${WORK}"

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

# Copies the tracked files of a corpus (git ls-files; everything for non-git trees, minus build
# output), never ignored runtime data; aborts below 4 GiB free disk space.
copy_corpus() {
  local source="$1" target="$2" free_kib
  free_kib="$(df -Pk "${REPO_ROOT}" | awk 'NR == 2 { print $4 }')"
  if (( free_kib < 4 * 1024 * 1024 )); then
    echo "$(basename "$0"): less than 4 GiB free, not copying ${source}" >&2
    return 1
  fi
  mkdir -p "${target}"
  if git -C "${source}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    # Symlinks (mode 120000) are skipped: broken ones abort openrsync. Exit 23/24 (partial
    # transfer, vanished files) still leaves a usable copy.
    local status=0
    git -C "${source}" ls-files -s | grep -v '^120000' | cut -f2- \
      | rsync -a --files-from=- "${source}/" "${target}/" || status=$?
    (( status == 0 || status == 23 || status == 24 )) || return 1
  else
    rsync -a --exclude .git --exclude target --exclude node_modules --exclude .cache "${source}/" "${target}/"
  fi
}

wanted=("$@")
selected() {
  (( ${#wanted[@]} == 0 )) && return 0
  local name
  for name in "${wanted[@]}"; do [[ "${name}" == "$1" ]] && return 0; done
  return 1
}

# Mean wall time in milliseconds of a command, from hyperfine's JSON export.
mean_ms() {
  local json="${WORK}/hyperfine.json"
  hyperfine -N --warmup 3 --runs 15 --export-json "${json}" "$@" >/dev/null 2>&1
  jq -r '.results[0].summary.time_wall_clock.mean * 10000 | round / 10' "${json}"
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
  build="$( { cd "${copy}" && /usr/bin/time -l graphify update . >/dev/null ; } 2>&1 )"
  printf ' %12s %9s %9s' \
    "$(awk '/ real / { printf "%.0f", $1 * 1000 }' <<<"${build}")" \
    "$(awk '/maximum resident set size/ { printf "%.0f", $1 / 1048576 }' <<<"${build}")" \
    "$(du -sk "${copy}/graphify-out" | awk '{ print $1 }')"
  printf '%s' "${edit}" >> "${target}"
  change="$( { cd "${copy}" && /usr/bin/time -l graphify update . >/dev/null ; } 2>&1 )"
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
  index="${WORK}/index-${name}"
  rm -rf "${index}"
  build="$( { /usr/bin/time -l "${BINARY}" index -C "${path}" --index-dir "${index}" >"${WORK}/out" ; } 2>&1 )"
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
    copy_corpus "${path}" "${copy}" || continue
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
