#!/usr/bin/env bash
# Compare edge depth with upstream Graphify on one corpus: which of Graphify's in-corpus
# relations (calls, inheritance, implementation, imports) xcalygraph also emits, and samples of
# the ones it misses, for manual verification against source. Graphify output is a candidate
# list, never evidence of correctness (AGENTS.md, Upstream reference).
# Usage: scripts/benchmarks/compare-depth.sh CORPUS [SAMPLES]
#   CORPUS: a directory under research/corpora/, or graphify for the upstream clone.
#   SAMPLES: misses listed per relation (default 15).
#   Works on a copy of the tracked files below .cache/depth/; Graphify's graph is reused while
#   the copy exists. Edges are matched by source file, source line (+-1) and target file;
#   imports by the file pair; a C-family header and its implementation count as one target.
# Requires: graphify, python3, rsync; the release test build is made here.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly REPO_ROOT
readonly WORK="${REPO_ROOT}/.cache/depth"

if (( $# < 1 || $# > 2 )); then
  echo "usage: compare-depth.sh CORPUS [SAMPLES]" >&2
  exit 2
fi
readonly CORPUS="$1"
readonly SAMPLES="${2:-15}"
for tool in graphify python3 rsync; do
  command -v "${tool}" >/dev/null || { echo "compare-depth.sh: ${tool} missing" >&2; exit 1; }
done

if [[ "${CORPUS}" == graphify ]]; then
  source_dir="${REPO_ROOT}/research/upstream/graphify"
else
  source_dir="${REPO_ROOT}/research/corpora/${CORPUS}"
fi
[[ -d "${source_dir}" ]] || { echo "compare-depth.sh: ${source_dir} missing" >&2; exit 1; }

copy="${WORK}/${CORPUS}"
if [[ ! -f "${copy}/graphify-out/graph.json" ]]; then
  rm -rf "${copy}"
  mkdir -p "${copy}"
  # Symlinks (mode 120000) are skipped; exit 23/24 (partial transfer) still leaves a usable copy.
  status=0
  git -C "${source_dir}" ls-files -s | grep -v '^120000' | cut -f2- \
    | rsync -a --files-from=- "${source_dir}/" "${copy}/" || status=$?
  (( status == 0 || status == 23 || status == 24 )) || exit 1
  (cd "${copy}" && graphify update . >/dev/null 2>&1) || true
  [[ -f "${copy}/graphify-out/graph.json" ]] || { echo "compare-depth.sh: graphify built no graph" >&2; exit 1; }
fi

ours="${WORK}/${CORPUS}-edges.tsv"
(
  cd "${REPO_ROOT}"
  XCALYGRAPH_DUMP_ROOT="${source_dir}" XCALYGRAPH_DUMP_FILE="${ours}" \
    cargo test -q --release --locked dump_edges -- --ignored >/dev/null
)

python3 -I - "${copy}/graphify-out/graph.json" "${ours}" "${SAMPLES}" <<'PYTHON'
import collections
import json
import sys

graph_path, ours_path, samples = sys.argv[1], sys.argv[2], int(sys.argv[3])
graph = json.load(open(graph_path))
nodes = {node["id"]: node for node in graph["nodes"]}

def line_of(location):
    if isinstance(location, str) and location.startswith("L"):
        digits = location[1:].split("-")[0]
        return int(digits) if digits.isdigit() else 0
    return 0

# Graphify relation -> xcalygraph edge kinds that express it.
families = {
    "calls": {"calls", "tests", "uses"},
    "inherits": {"inherits", "implements"},
    "implements": {"implements", "inherits"},
    "imports": {"imports", "uses", "references"},
}

def unit(path):
    # A C-family declaration and its definition (`x.h`, `x.m`) are one target.
    stem, dot, extension = path.rpartition(".")
    return stem if dot and extension in {"h", "hh", "hpp", "hxx", "c", "cc", "cpp", "cxx", "m", "mm"} else path

ours = collections.defaultdict(set)
ours_by_kind = collections.Counter()
with open(ours_path) as handle:
    for row in handle:
        kind, _, source, line, target, _, _ = row.rstrip("\n").split("\t")
        ours[(source, unit(target))].add((kind, int(line)))
        ours_by_kind[kind] += 1

def matched(relation, source, line, target):
    for kind, our_line in ours.get((source, unit(target)), ()):
        # File-level imports carry no line in xcalygraph, so only the file pair counts.
        if kind in families[relation] and (
            relation == "imports" or line == 0 or abs(our_line - line) <= 1
        ):
            return True
    return False

totals = collections.Counter()
hits = collections.Counter()
misses = collections.defaultdict(list)
for link in graph["links"]:
    relation = link.get("relation")
    if relation not in families:
        continue
    target = nodes.get(link["target"], {})
    source_file = link.get("source_file") or nodes.get(link["source"], {}).get("source_file")
    target_file = target.get("source_file")
    # Only in-corpus targets: external libraries are not graph nodes in xcalygraph.
    if not source_file or not target_file or source_file == target_file and relation == "imports":
        continue
    line = line_of(link.get("source_location"))
    key = (relation, link.get("confidence", "?"))
    totals[key] += 1
    if matched(relation, source_file, line, target_file):
        hits[key] += 1
    else:
        misses[relation].append(
            f"{source_file}:{line} -> {target.get('label')} @ {target_file}:{line_of(target.get('source_location'))}"
        )

print("graphify relation   confidence  in-corpus  matched")
for key in sorted(totals):
    share = 100 * hits[key] // max(totals[key], 1)
    print(f"{key[0]:<19} {key[1]:<11} {totals[key]:>9}  {hits[key]:>6} ({share}%)")
print("xcalygraph edges: " + ", ".join(f"{kind} {count}" for kind, count in sorted(ours_by_kind.items())))
for relation, rows in sorted(misses.items()):
    step = max(len(rows) // samples, 1)
    print(f"== missed {relation}: {len(rows)}")
    for row in rows[::step][:samples]:
        print("  " + row)
PYTHON
