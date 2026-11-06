#!/usr/bin/env bash
# Rank of the operation that answers behavior questions, in `find` output at equal budget.
# Usage: scripts/benchmarks/measure-behavior.sh BINARY [BUDGET]
# Cases: scripts/benchmarks/questions/behavior.tsv (corpus, question, implementing name, its file).
# A case counts when the name stands as a whole word on a symbol line of that file or in a chain.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
(( $# >= 1 && $# <= 2 )) && [[ -x "$1" ]] || { echo "usage: measure-behavior.sh BINARY [BUDGET]" >&2; exit 2; }
readonly binary="$1" budget="${2:-600}"
work="$(mktemp -d "${TMPDIR:-/tmp}/xcalygraph-behavior.XXXXXXXX")"
trap 'rm -rf -- "${work}"' EXIT
python3 -I - "${REPO_ROOT}" "${binary}" "${budget}" "${work}" <<'PY'
import os
import re
import subprocess
import sys

repo, binary, budget, work = sys.argv[1:]
environment = dict(os.environ, XCALYGRAPH_INDEX_DIR=work)
found = total = 0
for row in open(os.path.join(repo, "scripts/benchmarks/questions/behavior.tsv"), encoding="utf-8"):
    corpus, question, name, path = row.rstrip("\n").split("\t")
    root = os.path.join(repo, "research/corpora", corpus)
    output = subprocess.run([binary, "find", *question.split(), "-C", root, "-b", budget],
                            env=environment, capture_output=True, text=True).stdout
    short = name.rsplit(".", 1)[-1]
    word = re.compile(rf"(?<![\w]){re.escape(short)}(?!\w)")
    current, shown = None, False
    for line in output.splitlines():
        if line.startswith("~ "):
            # A chain names its sites as `NAME path:line`.
            shown = shown or re.search(rf"{re.escape(short)} {re.escape(path)}:\d+", line) is not None
        elif line and not line.startswith((" ", "!")):
            current = line.split()[0]
        elif current == path and re.match(r"\s+\d+ ", line) and word.search(line):
            shown = True
    found += shown
    total += 1
    print(f"{'shown' if shown else 'missing'}\t{corpus}\t{name}\t{question}")
print(f"shown {found} of {total}")
PY
