#!/usr/bin/env bash
# Gate of `hook prompt` on the sealed questions: how often it fires, how often a fired answer holds
# all required facts, how often unrelated prompts stay silent, and its latency.
# Usage: scripts/benchmarks/measure-hook-gate.sh BINARY ROOTS_TSV
#   ROOTS_TSV: corpus, revision, source digest, root per line (.cache/edge-baseline/roots.tsv).
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
(( $# == 2 )) && [[ -x "$1" && -f "$2" ]] || { echo "usage: measure-hook-gate.sh BINARY ROOTS_TSV" >&2; exit 2; }
work="$(mktemp -d "${REPO_ROOT}/.cache/hook-gate.XXXXXXXX")"
trap 'rm -rf -- "${work}"' EXIT
python3 -I - "${REPO_ROOT}" "$1" "$2" "${work}" <<'PYTHON'
import glob
import json
import os
import re
import statistics
import subprocess
import sys
import time

repo, binary, roots_file, work = sys.argv[1:]
binary = os.path.abspath(binary)
roots = {line.split("\t")[0]: line.rstrip("\n").split("\t")[3] for line in open(roots_file)}
negative_prompts = [line.strip() for line in open(f"{repo}/scripts/benchmarks/questions/hook-negative.tsv") if line.strip()]

def records(text):
    found, path = [], None
    for line in text.splitlines():
        match = re.match(r"  (\d+) (.*)", line)
        if match and path:
            found.append((path, int(match[1]), match[2]))
        elif line and not line.startswith((" ", "~ ", "+", "!")):
            path = line
    return found

def holds(found, locator):
    return any(path == locator["path"] and line == locator["line"]
               and re.search(r"(?<!\w)" + re.escape(locator["name"]) + r"(?!\w)", name)
               for path, line, name in found)

def ask(root, prompt, environment):
    start = time.perf_counter()
    done = subprocess.run([binary, "hook", "prompt"], input=json.dumps({"prompt": prompt, "cwd": root}),
                          env=environment, capture_output=True, text=True, cwd=root)
    elapsed = (time.perf_counter() - start) * 1000
    if done.returncode != 0:
        sys.exit(f"hook exit {done.returncode}")
    context = json.loads(done.stdout)["hookSpecificOutput"]["additionalContext"] if done.stdout else None
    return context, elapsed

positive = fired = complete = negative = spoken = chatter = chatter_spoken = 0
latencies = []
for path in sorted(glob.glob(f"{repo}/scripts/benchmarks/questions/sealed/*.jsonl")):
    corpus = os.path.basename(path)[:-6]
    root = roots.get(corpus)
    if corpus.startswith("bulletproof-react"):
        root = os.path.join(roots["bulletproof-react"], "apps/react-vite")
    if root is None or not os.path.isdir(root):
        continue
    environment = dict(os.environ, XCALYGRAPH_INDEX_DIR=os.path.join(work, corpus), HOME=work)
    subprocess.run([binary, "index", "-C", root], env=environment, capture_output=True, check=True)
    for line in open(path):
        case = json.loads(line)
        context, elapsed = ask(root, case["question"], environment)
        latencies.append(elapsed)
        if case["requires"]:
            positive += 1
            if context:
                fired += 1
                found = records(context)
                complete += all(any(holds(found, locator) for locator in group["alternatives"]) for group in case["requires"])
        else:
            negative += 1
            spoken += bool(context)
    for prompt in negative_prompts:
        context, elapsed = ask(root, prompt, environment)
        latencies.append(elapsed)
        chatter += 1
        chatter_spoken += bool(context)
latencies.sort()
print(f"positive {positive} fired {fired} ({fired / max(positive, 1):.0%}), complete when fired {complete} ({complete / max(fired, 1):.0%})")
print(f"sealed negatives {negative} spoke {spoken}; unrelated prompts {chatter} spoke {chatter_spoken} (silent {1 - chatter_spoken / max(chatter, 1):.0%})")
print(f"latency median {statistics.median(latencies):.1f} ms p95 {latencies[int(len(latencies) * 0.95)]:.1f} ms")
PYTHON
