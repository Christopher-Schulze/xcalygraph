#!/usr/bin/env bash
# Measure edge precision and recall against compiler indexes (SCIP) for Go, Rust, Python and
# TypeScript/JavaScript corpora; development evidence only, the product gains no dependency.
# Usage: scripts/benchmarks/measure-edge-oracle.sh CORPUS...
#   CORPUS is xcalygraph, graphify (the pinned upstream clone) or a pinned public corpus under
#   research/corpora/. The language comes from scripts/benchmarks/corpora.tsv (xcalygraph: rust,
#   graphify: python).
#   Indexers (development tools, on PATH or in SCIP_TOOLS, default ~/.local/share/scip-tools):
#   scip, scip-go, rust-analyzer, scip-python, scip-typescript.
#   Output: .cache/edge-oracle/run.*/ with per-corpus edges, oracle references and verdicts, plus
#   summary.tsv: corpus, language, kind, confidence, edges, correct, incorrect, unknown, precision,
#   and recall rows (recall-calls / recall-types: distinct source-file/target pairs the compiler
#   references, and how many have an edge; edges are deduplicated per source symbol and target,
#   so recall is measured per file and target rather than per line).
# Verdict per edge: unknown when the compiler has no symbol for the target; correct when the
# compiler resolves a reference on the edge line to the edge
# target (same file, same name, definition within 5 lines) or to a method the target implements;
# for inherits/implements, when the source type declared there has that relationship; incorrect
# when the compiler resolves in-repo references (or relationships) there but none is the target;
# unknown when the compiler reports nothing in-repo there.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
readonly TOOLS="${SCIP_TOOLS:-${HOME}/.local/share/scip-tools}"
(( $# > 0 )) || { echo "usage: measure-edge-oracle.sh CORPUS..." >&2; exit 2; }
PATH="${TOOLS}:${PATH}"
for tool in cargo git python3 scip; do
  command -v "${tool}" >/dev/null || { echo "measure-edge-oracle.sh: ${tool} is required" >&2; exit 1; }
done

language_of() {
  [[ "$1" == xcalygraph ]] && { echo rust; return; }
  [[ "$1" == graphify ]] && { echo python; return; }
  awk -F'\t' -v name="$1" '$1 == name { split($4, parts, ","); print parts[1] }' \
    "${REPO_ROOT}/scripts/benchmarks/corpora.tsv"
}

index_corpus() {
  local language="$1" root="$2" output="$3"
  case "${language}" in
    go) ( cd "${root}" && scip-go --output "${output}" >/dev/null 2>&1 ) ;;
    rust) ( cd "${root}" && rustup run "$(sed -n 's/^channel = "\(.*\)"/\1/p' "${REPO_ROOT}/rust-toolchain.toml")" \
              rust-analyzer scip . --output "${output}" >/dev/null 2>&1 ) ;;
    python) ( cd "${root}" && scip-python index . --output "${output}" >/dev/null 2>&1 ) ;;
    typescript|javascript)
      # A tsconfig that extends an uninstalled package fails; retry with an inferred config.
      ( cd "${root}" && scip-typescript index --infer-tsconfig --output "${output}" >/dev/null 2>&1 ) || {
        find "${root}" -maxdepth 1 -name 'tsconfig*.json' -exec mv {} {}.disabled \;
        ( cd "${root}" && scip-typescript index --infer-tsconfig --output "${output}" >/dev/null 2>&1 )
      } ;;
    *) echo "measure-edge-oracle.sh: no indexer for ${language}" >&2; return 1 ;;
  esac
}

mkdir -p "${REPO_ROOT}/.cache/edge-oracle"
WORK="$(mktemp -d "${REPO_ROOT}/.cache/edge-oracle/run.XXXXXXXX")"
readonly WORK
echo "evidence: ${WORK}" >&2
printf 'corpus\tlanguage\tkind\tconfidence\tedges\tcorrect\tincorrect\tunknown\tprecision\n' > "${WORK}/summary.tsv"
bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
for corpus in "$@"; do
  language="$(language_of "${corpus}")"
  [[ -n "${language}" ]] || { echo "measure-edge-oracle.sh: unknown corpus ${corpus}" >&2; exit 1; }
  source_root="${REPO_ROOT}/research/corpora/${corpus}"
  [[ "${corpus}" != xcalygraph ]] || source_root="${REPO_ROOT}"
  [[ "${corpus}" != graphify ]] || source_root="${REPO_ROOT}/research/upstream/graphify"
  [[ -d "${source_root}" ]] || { echo "measure-edge-oracle.sh: missing ${source_root}" >&2; exit 1; }
  # Both tools read the same committed tree.
  copy="${WORK}/${corpus}"
  mkdir "${copy}"
  git -C "${source_root}" archive --format=tar HEAD | tar -x -C "${copy}"
  index_corpus "${language}" "${copy}" "${WORK}/${corpus}.scip" || {
    echo "measure-edge-oracle.sh: ${corpus}: indexer failed" >&2
    exit 1
  }
  scip print --json "${WORK}/${corpus}.scip" > "${WORK}/${corpus}.scip.json"
  ( cd "${REPO_ROOT}" && XCALYGRAPH_DUMP_ROOT="${copy}" XCALYGRAPH_DUMP_FILE="${WORK}/${corpus}.edges.tsv" \
      cargo test -q --release --locked dump_edges -- --ignored >/dev/null )
  python3 -I - "${corpus}" "${language}" "${WORK}/${corpus}.scip.json" "${WORK}/${corpus}.edges.tsv" \
    "${WORK}/${corpus}.verdicts.tsv" >> "${WORK}/summary.tsv" <<'PYTHON'
import collections
import json
import re
import sys

corpus, language, scip_path, edges_path, verdicts_path = sys.argv[1:]
index = json.load(open(scip_path))
DEFINITION = 1

def start_line(occurrence):
    # 0-based start line from either the legacy `range` array or the typed range.
    if occurrence.get("range"):
        return occurrence["range"][0]
    for value in (occurrence.get("TypedRange") or {}).values():
        if value:
            return value.get("line", value.get("start_line", 0))
    return 0

def descriptor_name(symbol):
    # Last descriptor of a SCIP symbol: `pkg`/Type#method(). -> method
    # rust-analyzer: crate/impl#[`Index<'a>`]method(). ; scip-go: `pkg`/Type#Method().
    match = re.search(r"(`[^`]*`|[^\s#/.:`\[\]()!]+)(?:\([^()]*\))?[.#/:!]?$", symbol)
    return match.group(1).strip("`") if match else ""

# Generated files outside the corpus (Go build cache test mains) are not corpus source.
index["documents"] = [d for d in index["documents"] if not d["relative_path"].startswith("..")]
# A symbol can have several definitions: rust-analyzer gives same-named items in different
# integration-test crates one symbol, so a reference matches any of them.
definitions, implements = collections.defaultdict(set), collections.defaultdict(set)
for document in index["documents"]:
    for occurrence in document["occurrences"] or []:
        if occurrence.get("symbol_roles", 0) & DEFINITION and not occurrence["symbol"].startswith("local "):
            definitions[occurrence["symbol"]].add((document["relative_path"], start_line(occurrence) + 1))
    for information in document.get("symbols") or []:
        for relation in information.get("relationships") or []:
            if relation.get("is_implementation"):
                implements[information["symbol"]].add(relation["symbol"])

CONSTRUCTOR = re.compile(r"^(.*#)`?(?:<constructor>|__init__)`?\(\)\.$")
ACCESSOR = re.compile(r"`<[gs]et>")

def owner(symbol):
    # `new T()` / `T()` name the class in xcalygraph; the compiler names its constructor.
    match = CONSTRUCTOR.match(symbol)
    return match.group(1) if match and match.group(1) in definitions else symbol

references = collections.defaultdict(set)
for document in index["documents"]:
    for occurrence in document["occurrences"] or []:
        symbol = occurrence["symbol"]
        if occurrence.get("symbol_roles", 0) & DEFINITION or symbol not in definitions:
            continue
        references[(document["relative_path"], start_line(occurrence) + 1)].update({symbol, owner(symbol)})

by_location = collections.defaultdict(set)
for symbol, locations in definitions.items():
    for path, line in locations:
        by_location[(path, descriptor_name(symbol))].add((symbol, line))

def targets_of(path, name, line):
    return {symbol for symbol, defined in by_location.get((path, name), ()) if abs(defined - line) <= 5}

indexed = {document["relative_path"] for document in index["documents"]}
stats = collections.Counter()
with open(verdicts_path, "w") as verdicts:
    for row in open(edges_path, encoding="utf-8"):
        kind, confidence, source, line, target, target_line, name = row.rstrip("\n").split("\t")
        # Only edges from files the compiler indexed (the corpus language) can be judged.
        if kind not in ("calls", "uses", "tests", "inherits", "implements") or source not in indexed:
            continue
        confidence = "exact" if confidence == "1" else "inferred"
        expected = targets_of(target, name, int(target_line))
        if kind in ("inherits", "implements"):
            # The relationship belongs to the source type declared at the edge line.
            declared = {symbol for symbol, locations in definitions.items()
                        if any(path == source and abs(defined - int(line)) <= 2 for path, defined in locations)}
            oracle = {parent for symbol in declared for parent in implements.get(symbol, ())
                      if parent in definitions}
            dispatch = set()
        else:
            # Module and namespace symbols (`utils` -> file, scip-python's `pkg/__init__:` for a name
            # re-exported by the package) say nothing about the member called.
            oracle = {symbol for symbol in references.get((source, int(line)), set())
                      if not symbol.endswith(("/", "__init__:"))}
            if (kind in ("calls", "tests") and name not in {descriptor_name(symbol) for symbol in oracle}
                    and not any(symbol.endswith((").", "#")) for symbol in oracle)):
                # The compiler resolved no call on this line (only parameters or variables, e.g.
                # untyped pytest fixtures): it cannot judge the call edge.
                oracle = set()
            dispatch = {parent for symbol in expected for parent in implements.get(symbol, ())}
        names = {descriptor_name(symbol) for symbol in oracle}
        # A name read through a package's re-export reaches the compiler only as `pkg/__init__:`, so
        # the other symbols on that line cannot contradict the edge.
        reexported = any(symbol.endswith("__init__:") for symbol in references.get((source, int(line)), set()))
        # Without any in-repo reference there, or without a compiler symbol for the target (local
        # functions, CommonJS export objects), the compiler cannot judge the edge.
        if not oracle or not expected:
            verdict = "unknown"
        elif oracle & expected or oracle & dispatch:
            verdict = "correct"
        elif confidence == "inferred" and name in names:
            # Inferred dispatch: the compiler names the declared (trait/interface) method, the edge
            # names a same-named candidate implementation.
            verdict = "correct"
        elif name in names and any(symbol.endswith(name + ".") and not symbol.endswith(")." )
                                   for symbol in oracle):
            # Field shorthand (`Struct { name }`): the compiler reports only the field.
            verdict = "unknown"
        elif reexported:
            verdict = "unknown"
        else:
            verdict = "incorrect"
        stats[(kind, confidence, verdict)] += 1
        verdicts.write("\t".join([verdict, kind, confidence, source, line, target, target_line, name,
                                  ";".join(sorted(f"{path}:{defined}" for symbol in oracle
                                                  for path, defined in definitions[symbol]))]) + "\n")

for kind, confidence in sorted({(k, c) for k, c, _ in stats}):
    correct, incorrect, unknown = (stats[(kind, confidence, v)] for v in ("correct", "incorrect", "unknown"))
    judged = correct + incorrect
    precision = f"{100 * correct / judged:.1f}" if judged else "-"
    print(f"{corpus}\t{language}\t{kind}\t{confidence}\t{correct + incorrect + unknown}\t{correct}\t{incorrect}\t{unknown}\t{precision}")

# Recall: compiler references from the corpus to in-repo callables and types, against edges on
# the same line to the same target (any kind, any confidence).
edge_targets = collections.defaultdict(set)
for row in open(edges_path, encoding="utf-8"):
    kind, confidence, source, line, target, target_line, name = row.rstrip("\n").split("\t")
    for symbol in targets_of(target, name, int(target_line)):
        edge_targets[source].add(symbol)
        edge_targets[source].update(implements.get(symbol, ()))
pairs = set()
for (path, line), symbols in references.items():
    for symbol in symbols:
        if ACCESSOR.search(symbol) or owner(symbol) != symbol:
            continue  # property accessors are not calls; constructors count as their class
        group = "recall-calls" if symbol.endswith(").") else "recall-types" if symbol.endswith("#") else None
        if group is not None and (path, line) not in definitions[symbol]:
            pairs.add((group, path, symbol))
recall = collections.Counter()
with open(verdicts_path.replace(".verdicts.tsv", ".missed.tsv"), "w") as missed:
    for group, path, symbol in sorted(pairs):
        recall[(group, "total")] += 1
        if symbol in edge_targets.get(path, ()):
            recall[(group, "found")] += 1
        else:
            location = min(definitions[symbol])
            missed.write(f"{group}\t{path}\t{location[0]}:{location[1]}\t{symbol}\n")
for group in ("recall-calls", "recall-types"):
    total, found = recall[(group, "total")], recall[(group, "found")]
    if total:
        print(f"{corpus}\t{language}\t{group}\tall\t{total}\t{found}\t{total - found}\t0\t{100 * found / total:.1f}")
PYTHON
done
column -t -s $'\t' "${WORK}/summary.tsv"
( set -o noclobber; printf 'xcalygraph-cache-v1\n' > "${WORK}/.xcalygraph-disposable" )
