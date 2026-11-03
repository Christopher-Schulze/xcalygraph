#!/usr/bin/env bash
# Agent benchmark (docs/spec.md section 12): headless Codex (`codex exec`, model gpt-6-luna,
# reasoning effort max; owner decision 2026-11-03) answers each question in three arms (no tool,
# Graphify skill, xcalygraph skill); reports agent success and agent cost.
# Usage: scripts/benchmarks/run-agent-benchmark.sh [--run] [--runs N] CORPUS...
#   Without --run it prints the planned invocations only (dry run). Real runs use the owner's
#   ChatGPT login and need owner approval per milestone (AGENTS.md, Loop protocol).
#   Each arm uses its own temporary HOME and CODEX_HOME (only auth.json linked from the owner's
#   CODEX_HOME, default ~/.codex) and a git-archive copy of the corpus; PATH keeps no other arm's tool.
#   Status per answer: correct (expected file and symbol), wrong_symbol (file only), miss, error
#   (agent or API failure) or leak (the trace used another arm's tool). Cost: tokens (input,
#   output, reasoning), agent messages, tool calls and wall time from the JSONL trace, retained per
#   run under traces/.
# Requires: python3, seq, git, tar, codex; graphify for the Graphify arm; Cargo for a guarded release build.
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
readonly MODEL="gpt-6-luna" EFFORT="max"
readonly OWNER_CODEX_HOME="${CODEX_HOME:-${HOME}/.codex}"
while (( $# > 0 )); do
  case "$1" in
    --run) run=1; shift ;;
    --runs) (( $# >= 2 )) || { echo "run-agent-benchmark.sh: --runs needs a positive integer" >&2; exit 2; }; runs="$2"; shift 2 ;;
    *) break ;;
  esac
done
(( $# > 0 )) || { echo "usage: run-agent-benchmark.sh [--run] [--runs N] CORPUS..." >&2; exit 2; }
[[ "${runs}" =~ ^[0-9]+$ && "${runs}" =~ [1-9] ]] || {
  echo "run-agent-benchmark.sh: --runs needs a positive integer" >&2
  exit 2
}
run_attempts="$(seq 1 "${runs}")" || { echo "run-agent-benchmark.sh: cannot enumerate runs" >&2; exit 1; }
[[ -n "${run_attempts}" ]] || { echo "run-agent-benchmark.sh: cannot enumerate runs" >&2; exit 1; }
attempts=()
while IFS= read -r attempt; do
  [[ "${attempt}" =~ ^[0-9]+$ ]] || { echo "run-agent-benchmark.sh: invalid run enumeration" >&2; exit 1; }
  attempts+=("${attempt}")
done <<< "${run_attempts}"
if (( run )); then
  for tool in codex git tar; do
    command -v "${tool}" >/dev/null || { echo "run-agent-benchmark.sh: ${tool} missing" >&2; exit 1; }
  done
  [[ -f "${OWNER_CODEX_HOME}/auth.json" ]] || {
    echo "run-agent-benchmark.sh: no Codex login at ${OWNER_CODEX_HOME}/auth.json; run 'codex login'" >&2
    exit 1
  }
fi
agent_flags=(exec --json --ephemeral --skip-git-repo-check --dangerously-bypass-approvals-and-sandbox
  -m "${MODEL}" -c "model_reasoning_effort=\"${EFFORT}\"")

corpus_root() {
  local private
  case "$1" in
    xcalygraph) printf '%s' "${REPO_ROOT}" ;;
    graphify) printf '%s' "${REPO_ROOT}/research/upstream/graphify" ;;
    *) if [[ "$1" =~ ^[A-Za-z0-9._/-]+$ && "$1" != *..* && -d "${REPO_ROOT}/research/corpora/$1" ]]; then
         printf '%s' "${REPO_ROOT}/research/corpora/$1"
       elif [[ -f "${PRIVATE_LIST}" ]]; then
         private="$(bash "${REPO_ROOT}/scripts/utils/read-private-corpus.sh" get "${PRIVATE_LIST}" "$1" && printf '.')" || return 1
         private="${private%.}"
         if [[ "${private}" == -* ]]; then private="./${private}"; fi
         printf '%s' "${private}"
       fi ;;
  esac
}

# A corpus subdirectory such as bulletproof-react/apps/react-vite names its question set with dashes.
question_file() {
  local name="${1//\//-}"
  if [[ -f "${QUESTIONS}/${name}.tsv" ]]; then echo "${QUESTIONS}/${name}.tsv"; else echo "${QUESTIONS}/private/${name}.tsv"; fi
}

# Copies the committed corpus tree, so arms see neither local agent instructions nor build output.
snapshot_corpus() {
  local root="$1" destination="$2" prefix
  mkdir "${destination}" || return
  if prefix="$(git -C "${root}" rev-parse --show-prefix 2>/dev/null)" \
      && git -C "${root}" rev-parse --verify -q "HEAD:${prefix}" >/dev/null; then
    git -C "${root}" archive --format=tar "HEAD:${prefix}" | tar -x -C "${destination}" || return
  else
    tar -C "${root}" --exclude .git -cf - . | tar -x -C "${destination}" || return
  fi
  # A repository of its own, as agents normally see it; without it, tools that look for the
  # nearest ancestor with .git would index the checkout this benchmark runs in.
  git -C "${destination}" init -q && git -C "${destination}" add -A \
    && GIT_AUTHOR_NAME=benchmark GIT_AUTHOR_EMAIL=benchmark@example.invalid \
       GIT_COMMITTER_NAME=benchmark GIT_COMMITTER_EMAIL=benchmark@example.invalid \
       git -C "${destination}" -c commit.gpgsign=false commit -q -m snapshot || return
}

validate_questions() {
  python3 -I - "$1" <<'PYTHON'
from pathlib import Path
import sys

count = 0
for number, line in enumerate(Path(sys.argv[1]).read_bytes().split(b"\n"), 1):
    if not line:
        continue
    if line.count(b"\t") != 1:
        raise SystemExit(f"run-agent-benchmark.sh: invalid question record at line {number}")
    question, expected = line.split(b"\t")
    if question == b"question" and expected == b"expected":
        continue
    path, separator, name = expected.partition(b"::")
    if (not question.strip() or not path.strip() or (separator and not name.strip())
            or any(byte < 32 or byte == 127 for byte in question + expected)):
        raise SystemExit(f"run-agent-benchmark.sh: invalid question record at line {number}")
    count += 1
if not count:
    raise SystemExit("run-agent-benchmark.sh: no questions")
PYTHON
}

select_corpora() {
  local corpus root questions missing=0
  selected_corpora=()
  for corpus in "$@"; do
    root="$(corpus_root "${corpus}" && printf '.')" || return 1
    root="${root%.}"
    questions="$(question_file "${corpus}")"
    if [[ -n "${root}" && -d "${root}" && -f "${questions}" ]]; then
      validate_questions "${questions}" || return 1
      selected_corpora+=("${corpus}")
    else
      echo "run-agent-benchmark.sh: ${corpus}: no root or questions" >&2
      missing=1
    fi
  done
  (( ${#selected_corpora[@]} > 0 )) || { echo "run-agent-benchmark.sh: no runnable corpora" >&2; return 1; }
  (( ! missing )) || { echo "run-agent-benchmark.sh: incomplete corpus selection" >&2; return 1; }
}

score_answer() {
  local path="$1" answer="$2" root="$3" status
  [[ -n "${path//[[:space:]]/}" ]] || { echo "run-agent-benchmark.sh: empty expected path" >&2; return 2; }
  if python3 -I -c '
import os
import re
import sys

expected, root = sys.argv[1:]
answer = sys.stdin.read()
opening = "([{<\"`" + chr(39)
closing = ")]}>\"`" + chr(39)
line_suffix = re.compile(r":[0-9]+(?::[0-9]+)?")

def boundary(position):
    if position == len(answer) or answer[position].isspace() or answer[position] in closing:
        return True
    following = position + 1
    return (answer[position] in ".,;!?" and
            (following == len(answer) or answer[following].isspace() or answer[following] in closing))

def end_boundary(position):
    if boundary(position) or answer.startswith("::", position):
        return True
    if answer[position] != ":":
        return False
    if boundary(position + 1):
        return True
    line = line_suffix.match(answer, position)
    return line is not None and boundary(line.end())

candidates = [expected]
if not os.path.isabs(expected):
    if not expected.startswith("./"):
        candidates.append("./" + expected)
    for base in {os.path.abspath(root), os.path.realpath(root)}:
        absolute = os.path.join(base, expected)
        if os.path.commonpath((base, os.path.abspath(absolute))) == base:
            candidates.append(absolute)

matched = False
for candidate in candidates:
    start = 0
    while (start := answer.find(candidate, start)) != -1:
        left = start == 0 or answer[start - 1].isspace() or answer[start - 1] in opening
        if left and end_boundary(start + len(candidate)):
            matched = True
            break
        start += 1
    if matched:
        break
print(int(matched), end="")
' "${path}" "${root}" <<< "${answer}"; then
    return 0
  else
    status=$?
    echo "run-agent-benchmark.sh: answer matcher failed" >&2
    return "${status}"
  fi
}

create_run() {
  if [[ -L "${REPO_ROOT}/.cache" || -L "${WORK_BASE}" ]]; then
    echo "run-agent-benchmark.sh: benchmark cache must not be a symlink" >&2
    return 1
  fi
  mkdir -p "${WORK_BASE}" || return
  local work
  work="$(mktemp -d "${WORK_BASE}/run.XXXXXXXX")" || return
  ( set -o noclobber; printf 'corpus\tarm\trun\tquestion\tstatus\ttokens\tmessages\ttools\tms\treasoning\n' > "${work}/results.tsv" ) || return
  mkdir "${work}/traces" "${work}/corpora" || return
  printf '%s\n' "${work}"
}

# Prepares a fresh HOME of an arm: only that arm's skill is installed.
prepare_home() {
  local arm="$1" binary="${3:-}" home
  home="$(mktemp -d "$2/home-${arm}.XXXXXXXX")" || return
  mkdir "${home}/.codex" || return
  # The login is the only shared state; config, sessions, skills and memories stay per arm.
  if [[ -f "${OWNER_CODEX_HOME}/auth.json" ]]; then
    ln -s "${OWNER_CODEX_HOME}/auth.json" "${home}/.codex/auth.json" || return
  fi
  case "${arm}" in
    graphify) HOME="${home}" CODEX_HOME="${home}/.codex" graphify install --platform codex >/dev/null || return ;;
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
if hashlib.sha256((skill_dir / "SKILL.md").read_bytes()).hexdigest() != identity["skill_sha256"]:
    raise SystemExit("run-agent-benchmark.sh: installed skill changed")
PYTHON
}

invoke_agent() {
  local arm="$1" home="$2" binary="$3" directory=""
  shift 3
  if [[ "${arm}" == xcalygraph ]]; then directory="${binary%/*}"; fi
  python3 -I -c '
import os
import shutil
import sys

arm, home, directory = sys.argv[1:4]
if directory and os.pathsep in directory:
    raise SystemExit("run-agent-benchmark.sh: binary directory cannot be represented in PATH")
program = shutil.which(sys.argv[4])
if program is None:
    raise SystemExit("run-agent-benchmark.sh: " + sys.argv[4] + " missing")
environment = {key: value for key, value in os.environ.items()
               if not key.startswith(("BASH_FUNC_", "GIT_", "XCALYGRAPH_"))
               and key not in ("BASH_ENV", "ENV", "ZDOTDIR", "CDPATH")}
environment["HOME"] = home
environment["CODEX_HOME"] = os.path.join(home, ".codex")
environment["GIT_CONFIG_GLOBAL"] = os.devnull
environment["GIT_CONFIG_NOSYSTEM"] = "1"
# Only the arm tool may be reachable: drop PATH entries that provide another arm tool.
foreign = {"none": ("xcalygraph", "graphify"), "xcalygraph": ("xcalygraph", "graphify"),
           "graphify": ("xcalygraph",)}[arm]
entries = [entry for entry in environment.get("PATH", os.defpath).split(os.pathsep)
           if entry and not any(os.path.exists(os.path.join(entry, tool)) for tool in foreign)]
if directory:
    entries.insert(0, directory)
environment["PATH"] = os.pathsep.join(entries)
os.execve(program, [program] + sys.argv[5:], environment)
' "${arm}" "${home}" "${directory}" "$@"
}

# Reads one `codex exec --json` trace; prints status, tokens, agent messages, tool calls,
# milliseconds and reasoning tokens.
evaluate_trace() {
  python3 -I - "$@" <<'PYTHON'
import json
import re
import shlex
import sys

trace, arm, matched, expected, elapsed = sys.argv[1:6]
items, usage, failed = [], None, False
for line in open(trace, encoding="utf-8", errors="replace"):
    line = line.strip()
    if not line.startswith("{"):
        continue
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        continue
    kind = event.get("type", "")
    if kind == "item.completed" and isinstance(event.get("item"), dict):
        items.append(event["item"])
    elif kind == "turn.completed":
        usage = event.get("usage") or {}
    elif kind in ("turn.failed", "error"):
        failed = True
messages = [item.get("text") or "" for item in items if item.get("type") == "agent_message"]
tools = [item for item in items if item.get("type") not in ("agent_message", "reasoning")]
foreign = {"none": ("xcalygraph", "graphify"), "xcalygraph": ("graphify",), "graphify": ("xcalygraph",)}[arm]

def invokes_foreign(item):
    # A leak runs another arm's tool: its name in command position, not a path or argument that
    # merely contains it (the graphify corpus has a graphify/ directory).
    command = item.get("command") if isinstance(item, dict) else None
    if not isinstance(command, str):
        return any(tool in json.dumps(item) for tool in foreign)
    try:
        wrapper = shlex.split(command)
    except ValueError:
        wrapper = []
    if len(wrapper) == 3 and wrapper[0].rsplit("/", 1)[-1] in ("bash", "sh", "zsh") and wrapper[1] == "-lc":
        command = wrapper[2]
    for part in re.split(r"&&|\|\||[;|\n(){}`]|\$\(", command):
        words = [word.strip("'\"") for word in part.split()]
        while words and (re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", words[0]) or words[0] in
                         ("exec", "env", "command", "time", "nohup", "sudo", "xargs", "then", "do", "if", "!")):
            words.pop(0)
        if words and words[0].rsplit("/", 1)[-1] in foreign:
            return True
    return False
if failed or usage is None or not messages:
    status = "error"
elif any(invokes_foreign(item) for item in tools):
    status = "leak"
elif matched != "1":
    status = "miss"
else:
    name = re.split(r"\.|::", expected.partition("::")[2])[-1] if "::" in expected else ""
    answer = messages[-1]
    status = "correct" if not name or re.search(r"(?<![\w])" + re.escape(name) + r"(?![\w])", answer) else "wrong_symbol"
usage = usage or {}
tokens = sum(int(usage.get(key) or 0) for key in ("input_tokens", "output_tokens", "reasoning_output_tokens"))
print("\t".join([status, str(tokens), str(len(messages)), str(len(tools)), elapsed,
                  str(int(usage.get("reasoning_output_tokens") or 0))]))
PYTHON
}

# The final agent message of a `codex exec --json` trace.
answer_text() {
  python3 -I -c '
import json
import sys
answer = ""
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        continue
    item = event.get("item") if isinstance(event, dict) else None
    if event.get("type") == "item.completed" and isinstance(item, dict) and item.get("type") == "agent_message":
        answer = item.get("text") or ""
print(answer)
' "$1"
}

select_corpora "$@"
if (( run )); then
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
for corpus in "${selected_corpora[@]}"; do
  root="$(corpus_root "${corpus}" && printf '.')"
  root="${root%.}"
  questions="$(question_file "${corpus}")"
  [[ -d "${root}" && -f "${questions}" ]] || { echo "run-agent-benchmark.sh: ${corpus}: no root or questions" >&2; exit 1; }
  for arm in "${ARMS[@]}"; do
    home="${WORK}/home-${arm}.<new>"
    workspace="${WORK}/corpora/${corpus//\//-}-${arm}"
    if (( run )); then
      home="$(prepare_home "${arm}" "${WORK}" "${BINARY}")"
      if [[ "${arm}" == xcalygraph ]]; then
        verify_arm "${home}" "${BINARY}" "${IDENTITY}" --content
        last_xcalygraph_home="${home}"
      fi
      # One copy per arm: tool output written into the tree (graphify-out/) must not reach another arm.
      snapshot_corpus "${root}" "${workspace}" || { echo "run-agent-benchmark.sh: ${corpus}: snapshot failed" >&2; exit 1; }
    fi
    number=0
    while IFS=$'\t' read -r question expected || [[ -n "${question}${expected}" ]]; do
      [[ "${question}" == "question" && "${expected}" == "expected" || -z "${question}" ]] && continue
      number=$((number + 1))
      path="${expected%%::*}"
      [[ -n "${path//[[:space:]]/}" ]] || { echo "run-agent-benchmark.sh: empty expected path" >&2; exit 1; }
      for attempt in "${attempts[@]}"; do
        if (( ! run )); then
          printf 'HOME=%q codex %s %q  # cwd %s (git archive of %s), arm %s, run %d\n' \
            "${home}" "${agent_flags[*]}" "${question} ${PROMPT_SUFFIX}" "${workspace}" "${root}" "${arm}" "${attempt}"
          continue
        fi
        trace="${WORK}/traces/${corpus//\//-}-${arm}-q${number}-r${attempt}.jsonl"
        if [[ "${arm}" == xcalygraph ]]; then verify_arm "${home}" "${BINARY}" "${IDENTITY}"; fi
        # A failed agent process is recorded as status error; its trace stays for inspection.
        started="$(python3 -I -c 'import time; print(time.monotonic_ns() // 1000000)')"
        ( cd "${workspace}" && invoke_agent "${arm}" "${home}" "${BINARY}" codex "${agent_flags[@]}" \
            "${question} ${PROMPT_SUFFIX}" < /dev/null ) > "${trace}" 2> "${trace%.jsonl}.stderr" || true
        elapsed=$(( $(python3 -I -c 'import time; print(time.monotonic_ns() // 1000000)') - started ))
        if [[ "${arm}" == xcalygraph ]]; then verify_arm "${home}" "${BINARY}" "${IDENTITY}"; fi
        answer="$(answer_text "${trace}")"
        matched="$(score_answer "${path}" "${answer}" "${workspace}")" || exit 1
        measures="$(evaluate_trace "${trace}" "${arm}" "${matched}" "${expected}" "${elapsed}")" || exit 1
        printf '%s\t%s\t%d\t%s\t%s\n' "${corpus}" "${arm}" "${attempt}" "${question}" "${measures}" >> "${results}"
      done
    done < "${questions}"
  done
done
if (( run )); then
  if [[ -n "${last_xcalygraph_home}" ]]; then
    verify_arm "${last_xcalygraph_home}" "${BINARY}" "${IDENTITY}" --content
  fi
  python3 -I - "${results}" <<'PYTHON'
import collections
import csv
import sys

rows = list(csv.DictReader(open(sys.argv[1], encoding="utf-8"), delimiter="\t"))
groups = collections.defaultdict(list)
for row in rows:
    groups[(row["corpus"], row["arm"])].append(row)
print("corpus\tarm\truns\tcorrect\twrong_symbol\tmiss\terror\tleak\ttokens\tmessages\ttools\tseconds\treasoning")
for (corpus, arm), items in sorted(groups.items()):
    count = collections.Counter(item["status"] for item in items)
    valid = [item for item in items if item["status"] not in ("error", "leak")] or [None]
    mean = lambda key: sum(float(item[key]) for item in valid if item) / max(1, len([item for item in valid if item]))
    answered = max(1, len(items) - count["error"] - count["leak"])
    print(f"{corpus}\t{arm}\t{len(items)}\t{100 * count['correct'] / answered:.0f}%\t{count['wrong_symbol']}\t"
          f"{count['miss']}\t{count['error']}\t{count['leak']}\t{mean('tokens'):.0f}\t{mean('messages'):.1f}\t"
          f"{mean('tools'):.1f}\t{mean('ms') / 1000:.1f}\t{mean('reasoning'):.0f}")
PYTHON
fi
