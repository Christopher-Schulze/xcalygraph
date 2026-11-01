#!/usr/bin/env bash
# Agent benchmark (docs/spec.md section 12): headless Claude Code answers each question in three
# arms (no tool, Graphify skill, xcalygraph skill); reports agent success and agent cost.
# Usage: scripts/benchmarks/run-agent-benchmark.sh [--run] [--runs N] CORPUS...
#   Without --run it prints the planned invocations only (dry run). Real runs use the owner's
#   Claude subscription and need owner approval per milestone (AGENTS.md, Loop protocol).
#   Each corpus/arm uses its own temporary HOME.
#   Success: the final answer names the expected file. Cost: total tokens, turns and wall time
#   from --output-format json.
# Requires: claude, jq; graphify for the Graphify arm; Cargo for a guarded release build.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && printf '%s.' "$PWD")"
REPO_ROOT="${REPO_ROOT%.}"
readonly REPO_ROOT
readonly WORK_BASE="${REPO_ROOT}/.cache/agent-benchmark"
readonly QUESTIONS="${REPO_ROOT}/scripts/benchmarks/questions"
readonly PRIVATE_LIST="${QUESTIONS}/private/corpora.tsv"
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
if (( run )); then
  for tool in claude jq; do
    command -v "${tool}" >/dev/null || { echo "run-agent-benchmark.sh: ${tool} missing" >&2; exit 1; }
  done
fi

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

select_corpora() {
  local corpus root questions
  selected_corpora=()
  for corpus in "$@"; do
    root="$(corpus_root "${corpus}" || true)"
    questions="$(question_file "${corpus}")"
    if [[ -n "${root}" && -d "${root}" && -f "${questions}" ]]; then
      selected_corpora+=("${corpus}")
    else
      echo "run-agent-benchmark.sh: ${corpus}: no root or questions" >&2
    fi
  done
  (( ${#selected_corpora[@]} > 0 )) || { echo "run-agent-benchmark.sh: no runnable corpora" >&2; return 1; }
}

create_run() {
  if [[ -L "${REPO_ROOT}/.cache" || -L "${WORK_BASE}" ]]; then
    echo "run-agent-benchmark.sh: benchmark cache must not be a symlink" >&2
    return 1
  fi
  mkdir -p "${WORK_BASE}" || return
  local work
  work="$(mktemp -d "${WORK_BASE}/run.XXXXXXXX")" || return
  ( set -o noclobber; printf 'corpus\tarm\trun\tquestion\tsuccess\ttokens\tturns\tms\n' > "${work}/results.tsv" ) || return
  printf '%s\n' "${work}"
}

# Prepares a fresh HOME of an arm: only that arm's skill is installed.
prepare_home() {
  local arm="$1" binary="${3:-}" home
  home="$(mktemp -d "$2/home-${arm}.XXXXXXXX")" || return
  mkdir "${home}/.claude" || return
  case "${arm}" in
    graphify) HOME="${home}" CLAUDE_CONFIG_DIR="${home}/.claude" graphify install --platform claude >/dev/null || return ;;
    xcalygraph) HOME="${home}" "${binary}" install --skill-only >/dev/null || return ;;
  esac
  echo "${home}"
}

verify_arm() {
  python3 -I - "$1" "$2" "$3" "${4:-}" <<'PYTHON'
import hashlib
import json
from pathlib import Path
import stat
import sys

home, binary, identity_path = map(Path, sys.argv[1:4])
identity = json.loads(identity_path.read_text())
info = binary.lstat()
actual = [info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns,
          stat.S_IMODE(info.st_mode), info.st_nlink]
if not stat.S_ISREG(info.st_mode) or actual != identity["binary_stat"]:
    raise SystemExit("run-agent-benchmark.sh: captured binary changed")
if sys.argv[4] == "--content":
    digest = hashlib.sha256()
    with binary.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    if digest.hexdigest() != identity["binary_sha256"]:
        raise SystemExit("run-agent-benchmark.sh: captured binary content changed")
skill_dir = home / ".agents/skills/xcalygraph"
link = home / ".claude/skills/xcalygraph"
if (not link.is_symlink() or link.resolve() != skill_dir.resolve()
        or hashlib.sha256((skill_dir / "SKILL.md").read_bytes()).hexdigest() != identity["skill_sha256"]):
    raise SystemExit("run-agent-benchmark.sh: installed skill changed")
PYTHON
}

invoke_agent() {
  local arm="$1" home="$2" binary="$3" directory=""
  shift 3
  if [[ "${arm}" == xcalygraph ]]; then directory="${binary%/*}"; fi
  python3 -I -c '
import os
import sys

home, directory = sys.argv[1:3]
if directory and os.pathsep in directory:
    raise SystemExit("run-agent-benchmark.sh: binary directory cannot be represented in PATH")
environment = {key: value for key, value in os.environ.items()
               if not key.startswith("BASH_FUNC_") and key not in ("BASH_ENV", "ENV", "ZDOTDIR", "CDPATH")}
environment["HOME"] = home
environment["CLAUDE_CONFIG_DIR"] = os.path.join(home, ".claude")
if directory:
    environment["PATH"] = directory + os.pathsep + environment.get("PATH", os.defpath)
os.execvpe(sys.argv[3], sys.argv[3:], environment)
' "${home}" "${directory}" "$@"
}

if (( run )); then
  select_corpora "$@"
  set -- "${selected_corpora[@]}"
  WORK="$(create_run)"
  echo "evidence: ${WORK}" >&2
  [[ "${WORK}/bin" != *:* ]] || { echo "run-agent-benchmark.sh: binary directory cannot be represented in PATH" >&2; exit 1; }
  mkdir "${WORK}/bin"
  ( set -o noclobber; bash "${REPO_ROOT}/scripts/utils/guard-benchmark.sh" \
      bash "${REPO_ROOT}/scripts/utils/capture-benchmark-binary.sh" --release "${WORK}/bin/xcalygraph" \
      > "${WORK}/binary-identity.json" )
else
  WORK="${WORK_BASE}/run.<new>"
fi
readonly WORK
readonly BINARY="${WORK}/bin/xcalygraph"
readonly IDENTITY="${WORK}/binary-identity.json"
last_xcalygraph_home=""
results="${WORK}/results.tsv"
for corpus in "$@"; do
  root="$(corpus_root "${corpus}")"
  questions="$(question_file "${corpus}")"
  [[ -d "${root}" && -f "${questions}" ]] || { echo "run-agent-benchmark.sh: ${corpus}: no root or questions" >&2; continue; }
  for arm in "${ARMS[@]}"; do
    home="${WORK}/home-${arm}.<new>"
    if (( run )); then
      home="$(prepare_home "${arm}" "${WORK}" "${BINARY}")"
      if [[ "${arm}" == xcalygraph ]]; then
        verify_arm "${home}" "${BINARY}" "${IDENTITY}" --content
        last_xcalygraph_home="${home}"
      fi
    fi
    while IFS=$'\t' read -r question expected; do
      [[ "${question}" == "question" || -z "${question}" ]] && continue
      path="${expected%%::*}"
      for attempt in $(seq 1 "${runs}"); do
        if (( ! run )); then
          printf 'HOME=%q claude -p %q --output-format json  # cwd %s, arm %s, run %d\n' \
            "${home}" "${question} ${PROMPT_SUFFIX}" "${root}" "${arm}" "${attempt}"
          continue
        fi
        if [[ "${arm}" == xcalygraph ]]; then verify_arm "${home}" "${BINARY}" "${IDENTITY}"; fi
        if ! output="$(cd "${root}" && invoke_agent "${arm}" "${home}" "${BINARY}" claude -p "${question} ${PROMPT_SUFFIX}" --output-format json)"; then
          echo "run-agent-benchmark.sh: agent invocation failed; see ${WORK}" >&2
          exit 1
        fi
        if [[ "${arm}" == xcalygraph ]]; then verify_arm "${home}" "${BINARY}" "${IDENTITY}"; fi
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
  if [[ -n "${last_xcalygraph_home}" ]]; then
    verify_arm "${last_xcalygraph_home}" "${BINARY}" "${IDENTITY}" --content
  fi
  awk -F'\t' 'NR > 1 { key = $1 "\t" $2; n[key]++; s[key] += $5; t[key] += $6; u[key] += $7; m[key] += $8 }
    END { printf "corpus\tarm\truns\tsuccess\ttokens\tturns\tseconds\n";
          for (k in n) printf "%s\t%d\t%d%%\t%d\t%.1f\t%.1f\n", k, n[k], 100 * s[k] / n[k], t[k] / n[k], u[k] / n[k], m[k] / n[k] / 1000 }' "${results}" | sort
fi
