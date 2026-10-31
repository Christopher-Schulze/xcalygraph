#!/usr/bin/env bash
# Measure index build, freshness, single-file refresh and footprint on the reference corpora.
# Usage: scripts/benchmarks/run-benchmarks.sh [CORPUS...]
#   Corpora: this repository, the upstream clone (research/upstream/graphify), pinned public
#   corpora under research/corpora/ and private corpora listed as "name<TAB>path" in the
#   gitignored scripts/benchmarks/questions/private/corpora.tsv. CORPUS names restrict the run.
#   Corpora are read-only: the single-file refresh runs on a copy below .cache/.
# Requires: hyperfine, jq, rsync; the release build is made here.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly WORK="${REPO_ROOT}/.cache/benchmarks"
readonly BINARY="${REPO_ROOT}/target/release/xcalygraph"
readonly PRIVATE_LIST="${REPO_ROOT}/scripts/benchmarks/questions/private/corpora.tsv"
readonly COPY_LIMIT=20000

for tool in hyperfine jq rsync; do
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
    git -C "${source}" ls-files -z | rsync -a --from0 --files-from=- "${source}/" "${target}/"
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

printf '%-14s %8s %9s %9s %10s %10s %9s %9s\n' corpus files build_ms rss_mb fresh_ms change_ms index_kb write_kb
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
  if (( files <= COPY_LIMIT )); then
    copy="${WORK}/copy-${name}"
    copy_index="${WORK}/index-copy-${name}"
    rm -rf "${copy}" "${copy_index}"
    copy_corpus "${path}" "${copy}" || continue
    "${BINARY}" index -C "${copy}" --index-dir "${copy_index}" >/dev/null
    # A tracked source file, so ignore rules cannot hide the change.
    tracked="$(git -C "${path}" ls-files -- '*.go' '*.rs' '*.py' '*.ts' 2>/dev/null | awk 'NR == 1')"
    if [[ -n "${tracked}" ]]; then
      target="${copy}/${tracked}"
    else
      target="$(find "${copy}" -type f \( -name '*.go' -o -name '*.rs' -o -name '*.py' -o -name '*.ts' \) -print -quit)"
    fi
    if [[ -n "${target}" ]]; then
      sleep 2
      find "${copy_index}" -type f -exec stat -f '%i %z %N' {} + | sort > "${WORK}/snapshot"
      change_ms="$(mean_ms --prepare "printf '\n// bench\n' >> '${target}'" "${BINARY} index -C ${copy} --index-dir ${copy_index}")"
      printf '\n// bench\n' >> "${target}"
      find "${copy_index}" -type f -exec stat -f '%i %z %N' {} + | sort > "${WORK}/snapshot"
      printf '\n// bench\n' >> "${target}"
      "${BINARY}" index -C "${copy}" --index-dir "${copy_index}" >/dev/null
      write_kb="$(( $(bytes_rewritten "${copy_index}" "${WORK}/snapshot") / 1024 ))"
    fi
    rm -rf "${copy}" "${copy_index}"
  fi
  printf '%-14s %8s %9s %9s %10s %10s %9s %9s\n' \
    "${name}" "${files}" "${build_ms}" "${rss_mb}" "${fresh_ms}" "${change_ms}" "${index_kb}" "${write_kb}"
done
