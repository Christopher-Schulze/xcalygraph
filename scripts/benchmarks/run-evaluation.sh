#!/usr/bin/env bash
# Answer quality and tokens per question against the rg and upstream Graphify baselines
# (docs/spec.md section 12).
# Usage: scripts/benchmarks/run-evaluation.sh [--no-upstream] [--set NAME] [--phase final|development] [--budget N] [--reuse-graphify RUN] [--validate] [CORPUS...]
#   --set NAME: the question sets of scripts/benchmarks/questions/NAME/ (held-out rounds heldout,
#   heldout2, ...: written before a tuning round, evaluated once at its end).
#   Question sets: scripts/benchmarks/questions/<corpus>.tsv and, for private corpora,
#   scripts/benchmarks/questions/private/<corpus>.tsv; columns question, expected location as
#   path::name (or path). Corpus roots: this repository (xcalygraph), the upstream clone
#   (graphify), pinned public corpora under research/corpora/ (hono; a subdirectory such as
#   bulletproof-react/apps/react-vite names its question set with dashes) and private corpora
#   from scripts/benchmarks/questions/private/corpora.tsv.
#   Per corpus: hit@5 of find and ctx, find_any (the expected file anywhere in the find answer,
#   the criterion gfy_any applies to Graphify's longer answer), mean tokens per answer (bytes / 3),
#   the rg baseline
#   (output of rg for the two longest question words plus a full read of the expected file
#   when rg shows it) and the upstream baseline (graphify query on a graph of a copy below
#   .cache/). Misses are listed with the first result.
# Requires: jq, rg, python3; graphify for the upstream baseline.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly WORK_BASE="${REPO_ROOT}/.cache/evaluation"
readonly SNAPSHOTS="${REPO_ROOT}/.cache/corpus-snapshots"
readonly BINARY="${REPO_ROOT}/target/debug/xcalygraph"
QUESTIONS="${REPO_ROOT}/scripts/benchmarks/questions"
readonly PRIVATE_LIST="${QUESTIONS}/private/corpora.tsv"

upstream=1
phase=final
validate=0
budget_args=()
reuse_args=()
while [[ "${1:-}" == --* ]]; do
  case "$1" in
    --no-upstream) upstream=0 ;;
    --set) QUESTIONS="${QUESTIONS}/$2"; shift ;;
    --phase) phase="$2"; shift ;;
    --budget) budget_args=(--budget "$2"); shift ;;
    --reuse-graphify) reuse_args=(--reuse-graphify "$2"); shift ;;
    --validate) validate=1 ;;
    *) echo "run-evaluation.sh: unknown flag $1" >&2; exit 2 ;;
  esac
  shift
done
readonly QUESTIONS
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

if [[ ! -f "${QUESTIONS}/manifest.json" ]]; then
  if (( validate || ${#budget_args[@]} || ${#reuse_args[@]} )) || [[ "${phase}" != final ]]; then
    echo "run-evaluation.sh: --phase/--budget/--reuse-graphify/--validate require a manifest-backed question set" >&2
    exit 2
  fi
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
    [[ -n "${root}" && -d "${root}" && -n "${questions}" ]] || {
      echo "run-evaluation.sh: ${corpus}: missing corpus or questions" >&2
      exit 1
    }
    has_questions=0
    while IFS=$'\t' read -r question expected || [[ -n "${question}${expected}" ]]; do
      [[ "${question}" == "question" || -z "${question}" ]] && continue
      [[ -n "${expected}" ]] || { echo "run-evaluation.sh: ${corpus}: question has no expected location" >&2; exit 1; }
      has_questions=1
    done < "${questions}"
    (( has_questions )) || { echo "run-evaluation.sh: ${corpus}: no questions" >&2; exit 1; }
  done
fi
for tool in jq rg python3; do
  command -v "${tool}" >/dev/null || { echo "run-evaluation.sh: ${tool} missing" >&2; exit 1; }
done
if (( upstream )) && ! command -v graphify >/dev/null; then
  echo "run-evaluation.sh: graphify missing; use --no-upstream for an explicitly one-tool evaluation" >&2
  exit 1
fi

cd "${REPO_ROOT}"
build_binary() {
  bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
  RUSTFLAGS="-Dwarnings" cargo build --locked -q
}

if [[ -f "${QUESTIONS}/manifest.json" ]]; then
  (( upstream )) || { echo "run-evaluation.sh: sealed comparisons require Graphify" >&2; exit 2; }
  if (( ${#budget_args[@]} || ${#reuse_args[@]} )) && { [[ "${phase}" != development ]] || (( validate )); }; then
    echo "run-evaluation.sh: --budget and --reuse-graphify are restricted to development" >&2
    exit 2
  fi
  if (( validate )); then
    phase=validate
  else
    build_binary
  fi
  exec bash "${REPO_ROOT}/scripts/benchmarks/evaluate-sealed.sh" "${QUESTIONS}/manifest.json" "${phase}" "${BINARY}" "${budget_args[@]}" "${reuse_args[@]}" "$@"
fi
if [[ -L "${REPO_ROOT}/.cache" || -L "${WORK_BASE}" ]]; then
  echo "run-evaluation.sh: evaluation cache must not be a symlink" >&2
  exit 1
fi
build_binary
mkdir -p "${WORK_BASE}"
WORK="$(mktemp -d "${WORK_BASE}/run.XXXXXXXX")"
readonly WORK
exec 3>&1
exec > >(tee "${WORK}/results.txt")
tee_pid=$!
readonly tee_pid
readonly BENCHMARK_HOME="${WORK}/home"
mkdir -p "${BENCHMARK_HOME}"
sha256_file() {
  python3 -I -c 'import hashlib, pathlib, sys; print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())' "$1"
}
sha256_binary="$(sha256_file "${BINARY}")"
printf 'binary_sha256\t%s\n' "${sha256_binary}" > "${WORK}/identity.tsv"

tokens() {
  local bytes
  bytes="$(wc -c)"
  echo $(( (bytes + 2) / 3 ))
}

# Prints "<file hit> <symbol hit> <file anywhere>" for the JSON Lines records: the expected file
# among the first five (docs/spec.md section 12, hit@5), the expected symbol name in that file,
# and the expected file anywhere in the answer (Graphify's hit criterion).
json_hit() {
  local path="$1" name="$2"
  jq -r 'select(.t == "sym" or .t == "file") | [.path, (.name // "")] | @tsv' \
    | awk -F'\t' -v path="${path}" -v name="${name}" '
        NR <= 5 && $1 == path { file = 1 }
        NR <= 5 && $1 == path && name != "" && ($2 == name || $2 ~ ("(\\.|::)" name "$")) { symbol = 1 }
        $1 == path { anywhere = 1 }
        END { print file + 0, symbol + 0, anywhere + 0 }'
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

printf '%-12s %4s %7s %6s %8s %7s %8s %8s %6s %8s %6s %8s\n' \
  corpus n find@5 sym@5 find_any ctx@5 find_tok ctx_tok rg_hit rg_tok gfy_any gfy_tok
for corpus in "${corpora[@]}"; do
  root="$(corpus_root "${corpus}")"
  questions="$(question_file "${corpus}")"
  [[ -n "${root}" && -d "${root}" && -n "${questions}" ]] || { echo "run-evaluation.sh: ${corpus}: missing corpus or questions" >&2; exit 1; }
  snapshot_args=()
  (( upstream )) && snapshot_args+=(--graphify)
  root="$(env HOME="${BENCHMARK_HOME}" bash "${REPO_ROOT}/scripts/utils/prepare-corpus.sh" "${root}" "${SNAPSHOTS}" "${snapshot_args[@]}")"
  printf '%s\t%s\t%s\n' "${corpus}" "${root}" "$(sha256_file "${questions}")" >> "${WORK}/identity.tsv"
  index="${WORK}/index-${corpus//\//-}"
  misses="${WORK}/misses-${corpus//\//-}.txt"
  : > "${misses}"
  # A question without any match (exit 4) scores zero instead of aborting the run.
  query() { env HOME="${BENCHMARK_HOME}" "${BINARY}" "$@" -C "${root}" --index-dir "${index}" || (( $? == 4 )); }
  query index >/dev/null
  graph=""
  if (( upstream )); then
    graph="${root}/graphify-out/graph.json"
  fi
  n=0 find_hits=0 symbol_hits=0 any_hits=0 ctx_hits=0 find_tokens=0 ctx_tokens=0 rg_hits=0 rg_tokens=0 gfy_hits=0 gfy_tokens=0
  while IFS=$'\t' read -r question expected || [[ -n "${question}${expected}" ]]; do
    [[ "${question}" == "question" || -z "${question}" ]] && continue
    [[ -n "${expected}" ]] || { echo "run-evaluation.sh: ${corpus}: question has no expected location" >&2; exit 1; }
    n=$(( n + 1 ))
    path="${expected%%::*}"
    name=""
    [[ "${expected}" == *::* ]] && name="${expected#*::}"
    answer_dir="${WORK}/${corpus//\//-}/q-${n}"
    mkdir -p "${answer_dir}"
    printf '%s\t%s\n' "${question}" "${expected}" > "${answer_dir}/question.tsv"
    find_json="$(query find "${question}" --json -b 600 2>"${answer_dir}/find-json.stderr")"
    printf '%s\n' "${find_json}" > "${answer_dir}/find.jsonl"
    read -r hit symbol_hit any_hit < <(json_hit "${path}" "${name}" <<<"${find_json}")
    find_hits=$(( find_hits + hit ))
    symbol_hits=$(( symbol_hits + symbol_hit ))
    any_hits=$(( any_hits + any_hit ))
    find_text="$(query find "${question}" -b 600 2>"${answer_dir}/find-text.stderr")"
    printf '%s\n' "${find_text}" > "${answer_dir}/find.txt"
    find_tokens=$(( find_tokens + $(printf '%s\n' "${find_text}" | tokens) ))
    if (( hit == 0 )); then
      first="$(query find "${question}" -b 60 2>/dev/null | sed -n '1,2p' | tr '\n' ' ')"
      printf 'find miss: %s -> expected %s, got %s\n' "${question}" "${expected}" "${first}" >> "${misses}"
    fi
    ctx_json="$(query ctx "${question}" --json -b 2000 2>"${answer_dir}/ctx-json.stderr")"
    printf '%s\n' "${ctx_json}" > "${answer_dir}/ctx.jsonl"
    read -r ctx_hit _ _ < <(json_hit "${path}" "${name}" <<<"${ctx_json}")
    ctx_hits=$(( ctx_hits + ctx_hit ))
    ctx_text="$(query ctx "${question}" -b 2000 2>"${answer_dir}/ctx-text.stderr")"
    printf '%s\n' "${ctx_text}" > "${answer_dir}/ctx.txt"
    ctx_tokens=$(( ctx_tokens + $(printf '%s\n' "${ctx_text}" | tokens) ))
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
      gfy_status=0
      gfy_out="$(cd "${root}" && env HOME="${BENCHMARK_HOME}" PYTHONHASHSEED=0 PYTHONSAFEPATH=1 PYTHONDONTWRITEBYTECODE=1 graphify query "${question}" --graph "${graph}" 2>>"${WORK}/upstream-errors.log")" || gfy_status=$?
      printf '%s\n' "${gfy_out}" > "${answer_dir}/graphify.txt"
      printf '%s\n' "${gfy_status}" > "${answer_dir}/graphify.exit"
      # Graphify documents 3 as ambiguous and 4 as no matching node: valid misses, not runtime failures.
      (( gfy_status == 0 || gfy_status == 3 || gfy_status == 4 )) || exit "${gfy_status}"
      gfy_tokens=$(( gfy_tokens + $(printf '%s' "${gfy_out}" | tokens) ))
      if (( gfy_status == 0 )) && grep -q -F "${path}" <<<"${gfy_out}"; then
        gfy_hits=$(( gfy_hits + 1 ))
      fi
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
  (( n > 0 )) || { echo "run-evaluation.sh: ${corpus}: no questions" >&2; exit 1; }
  if [[ -n "${graph}" ]]; then
    gfy_rate="$(( 100 * gfy_hits / n ))%"
    gfy_mean=$(( gfy_tokens / n ))
  else
    gfy_rate="-"
    gfy_mean="-"
  fi
  printf '%-12s %4d %6d%% %5d%% %7d%% %6d%% %8d %8d %5d%% %8d %6s %8s\n' \
    "${corpus}" "${n}" $(( 100 * find_hits / n )) $(( 100 * symbol_hits / n )) $(( 100 * any_hits / n )) $(( 100 * ctx_hits / n )) \
    $(( find_tokens / n )) $(( ctx_tokens / n )) $(( 100 * rg_hits / n )) $(( rg_tokens / n )) \
    "${gfy_rate}" "${gfy_mean}"
  if (( impact_n > 0 )); then
    printf '%-12s impact recall %d%% of %d, %d tokens per answer\n' \
      "${corpus}" $(( 100 * impact_hits / impact_n )) "${impact_n}" $(( impact_tokens / impact_n ))
  fi
done
echo "evidence: ${WORK} (identity.tsv, misses-<corpus>.txt, upstream-errors.log)"
exec >&3 3>&-
wait "${tee_pid}"
( set -o noclobber; printf 'xcalygraph-cache-v1\n' > "${WORK}/.xcalygraph-disposable" )
