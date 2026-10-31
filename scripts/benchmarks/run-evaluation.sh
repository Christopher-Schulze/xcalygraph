#!/usr/bin/env bash
# Answer quality and tokens per question against the rg and upstream Graphify baselines
# (docs/spec.md section 12).
# Usage: scripts/benchmarks/run-evaluation.sh [--no-upstream] [CORPUS...]
#   Question sets: scripts/benchmarks/questions/<corpus>.tsv and, for private corpora,
#   scripts/benchmarks/questions/private/<corpus>.tsv; columns question, expected location as
#   path::name (or path). Corpus roots: this repository (xcalygraph), the upstream clone
#   (graphify), pinned public corpora under research/corpora/ (hono; a subdirectory such as
#   bulletproof-react/apps/react-vite names its question set with dashes) and private corpora
#   from scripts/benchmarks/questions/private/corpora.tsv.
#   Per corpus: hit@5 of find and ctx, mean tokens per answer (bytes / 3), the rg baseline
#   (output of rg for the two longest question words plus a full read of the expected file
#   when rg shows it) and the upstream baseline (graphify query on a graph of a copy below
#   .cache/). Misses are listed with the first result.
# Requires: jq, rg; graphify for the upstream baseline.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly WORK="${REPO_ROOT}/.cache/evaluation"
readonly BINARY="${REPO_ROOT}/target/release/xcalygraph"
readonly QUESTIONS="${REPO_ROOT}/scripts/benchmarks/questions"
readonly PRIVATE_LIST="${QUESTIONS}/private/corpora.tsv"

upstream=1
if [[ "${1:-}" == "--no-upstream" ]]; then
  upstream=0
  shift
fi
for tool in jq rg; do
  command -v "${tool}" >/dev/null || { echo "run-evaluation.sh: ${tool} missing" >&2; exit 1; }
done
if (( upstream )) && ! command -v graphify >/dev/null; then
  echo "run-evaluation.sh: graphify missing; running without the upstream baseline" >&2
  upstream=0
fi

cd "${REPO_ROOT}"
cargo build --release --locked -q
mkdir -p "${WORK}"

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

corpus_root() {
  case "$1" in
    xcalygraph) echo "${REPO_ROOT}" ;;
    graphify) echo "${REPO_ROOT}/research/upstream/graphify" ;;
    *)
      if [[ -d "${REPO_ROOT}/research/corpora/$1" ]]; then
        echo "${REPO_ROOT}/research/corpora/$1"
      elif [[ -f "${PRIVATE_LIST}" ]]; then
        awk -F'\t' -v name="$1" '$1 == name { print $2 }' "${PRIVATE_LIST}"
      fi
      ;;
  esac
}

question_file() {
  local name="${1//\//-}"
  if [[ -f "${QUESTIONS}/${name}.tsv" ]]; then
    echo "${QUESTIONS}/${name}.tsv"
  elif [[ -f "${QUESTIONS}/private/${name}.tsv" ]]; then
    echo "${QUESTIONS}/private/${name}.tsv"
  fi
}

tokens() {
  local bytes
  bytes="$(wc -c)"
  echo $(( (bytes + 2) / 3 ))
}

# Prints "<file hit> <symbol hit>" for the first five JSON Lines records: the expected file
# (docs/spec.md section 12, hit@5), and the expected symbol name in that file.
json_hit() {
  local path="$1" name="$2"
  jq -r 'select(.t == "sym" or .t == "file") | [.path, (.name // "")] | @tsv' \
    | head -5 \
    | awk -F'\t' -v path="${path}" -v name="${name}" '
        $1 == path { file = 1 }
        $1 == path && name != "" && ($2 == name || $2 ~ ("(\\.|::)" name "$")) { symbol = 1 }
        END { print file + 0, symbol + 0 }'
}

stopword() {
  case "$1" in
    a|an|and|are|as|at|be|by|does|do|for|from|how|in|is|it|of|on|or|the|that|this|to|what|where|which|who|why|with|when|into|than|then|its|their|one|each|every|only|never|without) return 0 ;;
  esac
  return 1
}

# The two longest non-stopword words of a question.
rg_terms() {
  local word
  for word in $1; do
    word="$(tr -cd '[:alnum:]_' <<<"${word}")"
    (( ${#word} >= 3 )) || continue
    stopword "$(tr '[:upper:]' '[:lower:]' <<<"${word}")" && continue
    printf '%d %s\n' "${#word}" "${word}"
  done | sort -rn -k1,1 -s | awk 'NR <= 2 { print $2 }'
}

printf '%-12s %4s %7s %6s %7s %8s %8s %6s %8s %6s %8s\n' \
  corpus n find@5 sym@5 ctx@5 find_tok ctx_tok rg_hit rg_tok gfy@5 gfy_tok
corpora=("$@")
if (( ${#corpora[@]} == 0 )); then
  corpora=(xcalygraph graphify hono)
  if [[ -f "${PRIVATE_LIST}" ]]; then
    while IFS=$'\t' read -r name _; do
      [[ -n "${name}" && "${name}" != \#* ]] && corpora+=("${name}")
    done < "${PRIVATE_LIST}"
  fi
fi

for corpus in "${corpora[@]}"; do
  root="$(corpus_root "${corpus}")"
  questions="$(question_file "${corpus}")"
  [[ -n "${root}" && -d "${root}" && -n "${questions}" ]] || continue
  index="${WORK}/index-${corpus//\//-}"
  misses="${WORK}/misses-${corpus//\//-}.txt"
  : > "${misses}"
  # A question without any match (exit 4) scores zero instead of aborting the run.
  query() { "${BINARY}" "$@" -C "${root}" --index-dir "${index}" || (( $? == 4 )); }
  query index >/dev/null
  graph=""
  if (( upstream )); then
    copy="${WORK}/upstream-${corpus//\//-}"
    if [[ ! -f "${copy}/graphify-out/graph.json" ]]; then
      rm -rf "${copy}"
      copy_corpus "${root}" "${copy}" || continue
      (cd "${copy}" && graphify update . >/dev/null 2>&1) || true
    fi
    [[ -f "${copy}/graphify-out/graph.json" ]] && graph="${copy}/graphify-out/graph.json"
  fi
  n=0 find_hits=0 symbol_hits=0 ctx_hits=0 find_tokens=0 ctx_tokens=0 rg_hits=0 rg_tokens=0 gfy_hits=0 gfy_tokens=0
  while IFS=$'\t' read -r question expected; do
    [[ "${question}" == "question" || -z "${question}" ]] && continue
    n=$(( n + 1 ))
    path="${expected%%::*}"
    name=""
    [[ "${expected}" == *::* ]] && name="${expected#*::}"
    read -r hit symbol_hit < <(query find "${question}" --json -b 600 2>/dev/null | json_hit "${path}" "${name}" || echo "0 0")
    find_hits=$(( find_hits + hit ))
    symbol_hits=$(( symbol_hits + symbol_hit ))
    find_tokens=$(( find_tokens + $(query find "${question}" -b 600 2>/dev/null | tokens) ))
    if (( hit == 0 )); then
      first="$(query find "${question}" -b 60 2>/dev/null | sed -n '1,2p' | tr '\n' ' ')"
      printf 'find miss: %s -> expected %s, got %s\n' "${question}" "${expected}" "${first}" >> "${misses}"
    fi
    read -r ctx_hit _ < <(query ctx "${question}" --json -b 2000 2>/dev/null | json_hit "${path}" "${name}" || echo "0 0")
    ctx_hits=$(( ctx_hits + ctx_hit ))
    ctx_tokens=$(( ctx_tokens + $(query ctx "${question}" -b 2000 2>/dev/null | tokens) ))
    terms=()
    while IFS= read -r term; do terms+=(-e "${term}"); done < <(rg_terms "${question}")
    rg_out=""
    if (( ${#terms[@]} > 0 )); then
      rg_out="$(cd "${root}" && rg -n -i --no-heading --max-columns 200 "${terms[@]}" . 2>/dev/null || true)"
    fi
    spent="$(printf '%s' "${rg_out}" | tokens)"
    if grep -q -F "${path}:" <<<"${rg_out}"; then
      rg_hits=$(( rg_hits + 1 ))
      spent=$(( spent + $(tokens < "${root}/${path}") ))
    fi
    rg_tokens=$(( rg_tokens + spent ))
    if [[ -n "${graph}" ]]; then
      gfy_out="$(cd "$(dirname "$(dirname "${graph}")")" && graphify query "${question}" --graph "${graph}" 2>/dev/null || true)"
      gfy_tokens=$(( gfy_tokens + $(printf '%s' "${gfy_out}" | tokens) ))
      grep -q -F "${path}" <<<"${gfy_out}" && gfy_hits=$(( gfy_hits + 1 ))
    fi
  done < "${questions}"
  # Impact recall: the expected dependent among the affected lines of `impact NAME`.
  impact_set="${questions%.tsv}-impact.tsv"
  impact_n=0 impact_hits=0 impact_tokens=0
  if [[ -f "${impact_set}" ]]; then
    while IFS=$'\t' read -r target dependent; do
      [[ "${target}" == "name" || -z "${target}" ]] && continue
      impact_n=$(( impact_n + 1 ))
      dep_path="${dependent%%::*}"
      dep_name="${dependent#*::}"
      found="$(query impact "${target}" --json 2>/dev/null \
        | jq -r 'select(.rel == "in") | [.path, .name] | @tsv' \
        | awk -F'\t' -v path="${dep_path}" -v name="${dep_name}" '$1 == path && ($2 == name || $2 ~ ("(\\.|::)" name "$")) { hit = 1 } END { print hit + 0 }')"
      impact_hits=$(( impact_hits + found ))
      impact_tokens=$(( impact_tokens + $(query impact "${target}" 2>/dev/null | tokens) ))
      (( found == 1 )) || printf 'impact miss: %s -> expected dependent %s\n' "${target}" "${dependent}" >> "${misses}"
    done < "${impact_set}"
  fi
  (( n > 0 )) || continue
  if [[ -n "${graph}" ]]; then
    gfy_rate="$(( 100 * gfy_hits / n ))%"
    gfy_mean=$(( gfy_tokens / n ))
  else
    gfy_rate="-"
    gfy_mean="-"
  fi
  printf '%-12s %4d %6d%% %5d%% %6d%% %8d %8d %5d%% %8d %6s %8s\n' \
    "${corpus}" "${n}" $(( 100 * find_hits / n )) $(( 100 * symbol_hits / n )) $(( 100 * ctx_hits / n )) \
    $(( find_tokens / n )) $(( ctx_tokens / n )) $(( 100 * rg_hits / n )) $(( rg_tokens / n )) \
    "${gfy_rate}" "${gfy_mean}"
  if (( impact_n > 0 )); then
    printf '%-12s impact recall %d%% of %d, %d tokens per answer\n' \
      "${corpus}" $(( 100 * impact_hits / impact_n )) "${impact_n}" $(( impact_tokens / impact_n ))
  fi
done
echo "misses: ${WORK}/misses-<corpus>.txt"
