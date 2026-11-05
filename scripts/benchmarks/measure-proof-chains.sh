#!/usr/bin/env bash
# Count the required links of multi-step questions that a `find` answer shows on its own.
# Usage: scripts/benchmarks/measure-proof-chains.sh BINARY [BUDGET]
# Cases: scripts/benchmarks/questions/proof-chains.tsv (corpus, question, A>B links). A link counts
# when A and B stand next to each other in a `~` chain line, or B is a neighbour of top hit A.
# Output: one line per case and the totals; every link is an edge the source confirms.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
(( $# >= 1 && $# <= 2 )) && [[ -x "$1" ]] || { echo "usage: measure-proof-chains.sh BINARY [BUDGET]" >&2; exit 2; }
readonly binary="$1" budget="${2:-600}"
work="$(mktemp -d "${TMPDIR:-/tmp}/xcalygraph-chains.XXXXXXXX")"
trap 'rm -rf -- "${work}"' EXIT

python3 -I - "${REPO_ROOT}" "${binary}" "${budget}" "${work}" <<'PY'
import os
import re
import subprocess
import sys

repo, binary, budget, work = sys.argv[1:]
environment = dict(os.environ, XCALYGRAPH_INDEX_DIR=work)
shown = required = tokens = 0
for row in open(os.path.join(repo, "scripts/benchmarks/questions/proof-chains.tsv"), encoding="utf-8"):
    corpus, question, links = row.rstrip("\n").split("\t")
    root = os.path.join(repo, "research/corpora", corpus)
    output = subprocess.run([binary, "find", *question.split(), "-C", root, "-b", budget],
                            env=environment, capture_output=True, text=True, check=True).stdout
    chains = [re.findall(r"(?:^~ | >\d+\??  ?)(\S+) \S*:\d+", line)
              for line in output.splitlines() if line.startswith("~ ")]
    neighbours = re.findall(r"^\s+> (\S+)", output, re.M)
    first = re.search(r"^  \d+ .*", output, re.M)
    found = 0
    for link in links.split():
        source, target = link.split(">")
        adjacent = any(source == chain[i] and target == chain[i + 1]
                       for chain in chains for i in range(len(chain) - 1))
        leading = (first is not None and source.split(".")[-1].split("::")[-1] in first[0]
                   and target in neighbours)
        found += adjacent or leading
    shown += found
    required += len(links.split())
    tokens += (len(output) + 2) // 3
    print(f"{corpus}\t{found}/{len(links.split())}\t{question}")
print(f"links shown {shown} of {required}, mean tokens {tokens // 12}")
PY
