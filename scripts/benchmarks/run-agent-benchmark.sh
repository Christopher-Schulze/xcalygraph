#!/usr/bin/env bash
# Agent benchmark (docs/spec.md section 12): headless Claude Code answers each question in three
# arms (no tool, Graphify skill, xcalygraph skill); reports agent success and agent cost.
# Usage: scripts/benchmarks/run-agent-benchmark.sh [--run] [--runs N] CORPUS...
#   Without --run it prints the planned invocations only (dry run). Real runs use the owner's
#   Claude subscription and need owner approval per milestone (AGENTS.md, Loop protocol).
#   Each arm runs with its own temporary HOME, so only that arm's skill is visible.
#   Success: the final answer names the expected file. Cost: total tokens, turns and wall time
#   from --output-format json.
# Requires: claude, jq; graphify for the Graphify arm; an installed xcalygraph release build.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly WORK="${REPO_ROOT}/.cache/agent-benchmark"
readonly QUESTIONS="${REPO_ROOT}/scripts/benchmarks/questions"
readonly PRIVATE_LIST="${QUESTIONS}/private/corpora.tsv"
readonly BINARY="${REPO_ROOT}/target/release/xcalygraph"
readonly ARMS=(none graphify xcalygraph)
readonly PROMPT_SUFFIX="Answer with the file path and the symbol name that answer the question, then stop."

run=0
runs=3
while (( $# > 0 )); do
  case "$1" in
    --run) run=1; shift ;;
    --runs) runs="$2"; shift 2 ;;
    *) break ;;
  esac
done
(( $# > 0 )) || { echo "usage: run-agent-benchmark.sh [--run] [--runs N] CORPUS..." >&2; exit 2; }
for tool in claude jq; do
  command -v "${tool}" >/dev/null || { echo "run-agent-benchmark.sh: ${tool} missing" >&2; exit 1; }
done

corpus_root() {
  case "$1" in
    xcalygraph) echo "${REPO_ROOT}" ;;
    graphify) echo "${REPO_ROOT}/research/upstream/graphify" ;;
    *) [[ -f "${PRIVATE_LIST}" ]] && awk -F'\t' -v name="$1" '$1 == name { print $2 }' "${PRIVATE_LIST}" ;;
  esac
}

question_file() {
  if [[ -f "${QUESTIONS}/$1.tsv" ]]; then echo "${QUESTIONS}/$1.tsv"; else echo "${QUESTIONS}/private/$1.tsv"; fi
}

# Prepares the temporary HOME of an arm: only that arm's skill is installed.
prepare_home() {
  local arm="$1" home="${WORK}/home-$1"
  rm -rf "${home}"
  mkdir -p "${home}/.claude"
  case "${arm}" in
    graphify) HOME="${home}" graphify install --platform claude >/dev/null ;;
    xcalygraph) HOME="${home}" "${BINARY}" install --link claude >/dev/null ;;
  esac
  echo "${home}"
}

mkdir -p "${WORK}"
results="${WORK}/results.tsv"
(( run )) && printf 'corpus\tarm\trun\tquestion\tsuccess\ttokens\tturns\tms\n' > "${results}"
for corpus in "$@"; do
  root="$(corpus_root "${corpus}")"
  questions="$(question_file "${corpus}")"
  [[ -d "${root}" && -f "${questions}" ]] || { echo "run-agent-benchmark.sh: ${corpus}: no root or questions" >&2; continue; }
  for arm in "${ARMS[@]}"; do
    home="${WORK}/home-${arm}"
    (( run )) && home="$(prepare_home "${arm}")"
    while IFS=$'\t' read -r question expected; do
      [[ "${question}" == "question" || -z "${question}" ]] && continue
      path="${expected%%::*}"
      for attempt in $(seq 1 "${runs}"); do
        if (( ! run )); then
          printf 'HOME=%s claude -p %q --output-format json  # cwd %s, arm %s, run %d\n' \
            "${home}" "${question} ${PROMPT_SUFFIX}" "${root}" "${arm}" "${attempt}"
          continue
        fi
        output="$(cd "${root}" && HOME="${home}" claude -p "${question} ${PROMPT_SUFFIX}" --output-format json 2>/dev/null || echo '{}')"
        answer="$(jq -r '.result // ""' <<<"${output}")"
        success=0
        grep -q -F "${path}" <<<"${answer}" && success=1
        tokens="$(jq -r '((.usage.input_tokens // 0) + (.usage.output_tokens // 0) + (.usage.cache_read_input_tokens // 0) + (.usage.cache_creation_input_tokens // 0))' <<<"${output}")"
        turns="$(jq -r '.num_turns // 0' <<<"${output}")"
        ms="$(jq -r '.duration_ms // 0' <<<"${output}")"
        printf '%s\t%s\t%d\t%s\t%d\t%s\t%s\t%s\n' "${corpus}" "${arm}" "${attempt}" "${question}" "${success}" "${tokens}" "${turns}" "${ms}" >> "${results}"
      done
    done < "${questions}"
  done
done
if (( run )); then
  awk -F'\t' 'NR > 1 { key = $1 "\t" $2; n[key]++; s[key] += $5; t[key] += $6; u[key] += $7; m[key] += $8 }
    END { printf "corpus\tarm\truns\tsuccess\ttokens\tturns\tseconds\n";
          for (k in n) printf "%s\t%d\t%d%%\t%d\t%.1f\t%.1f\n", k, n[k], 100 * s[k] / n[k], t[k] / n[k], u[k] / n[k], m[k] / n[k] / 1000 }' "${results}" | sort
fi
