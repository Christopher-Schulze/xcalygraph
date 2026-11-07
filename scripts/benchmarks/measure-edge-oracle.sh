#!/usr/bin/env bash
# Measure edge precision and recall against compiler indexes (SCIP) for Go, Rust, Python and
# TypeScript/JavaScript corpora; development evidence only, the product gains no dependency.
# Usage: scripts/benchmarks/measure-edge-oracle.sh CORPUS...
#   CORPUS is xcalygraph, graphify (the pinned upstream clone) or a pinned public corpus under
#   research/corpora/. The language comes from scripts/benchmarks/corpora.tsv (xcalygraph: rust,
#   graphify: python). A TypeScript corpus without a root tsconfig.json is a monorepo: one
#   compiler project per nested tsconfig.json.
#   Indexers (development tools, on PATH or in SCIP_TOOLS, default ~/.local/share/scip-tools):
#   scip, scip-go, rust-analyzer, scip-python, scip-typescript.
#   Output: .cache/edge-oracle/run.*/ with per-corpus edges, oracle references and verdicts, plus
#   summary.tsv: corpus, language, kind, confidence, edges, correct, incorrect, unknown, precision,
#   target (the compiler names the target on that line, the caller's ownership stays unproven),
#   agreement (share of correct plus target among correct, incorrect and target),
#   and recall rows (recall-calls / recall-types / recall-variants: distinct source-file/target pairs the compiler
#   references, and how many have an edge; edges are deduplicated per source symbol and target,
#   so recall is measured per file and target rather than per line).
# Missed-pair rows retain group, source, target, compiler symbol and sorted one-based reference
# lines (comma-separated), so source classification remains possible after raw SCIP compaction.
# Verdict per edge: unknown when the compiler has no symbol for the target; correct when the
# compiler resolves a reference owned by the declared caller on the edge line to the edge
# target (same file, name and definition line, identifying one physical definition) or to a method
# the target implements; ambiguous locations and unsupported name-only dispatch stay unknown;
# for inherits/implements, when the source type declared there has that relationship; incorrect
# when the compiler resolves in-repo references (or relationships) there but none is the target;
# unknown when caller ranges or unambiguous targets are unavailable. The native edge dump
# appends caller declaration line, name and node kind after its seven-column depth prefix.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
readonly TOOLS="${SCIP_TOOLS:-${HOME}/.local/share/scip-tools}"
(( $# > 0 )) || { echo "usage: measure-edge-oracle.sh CORPUS..." >&2; exit 2; }
PATH="${TOOLS}:${PATH}"
for tool in cargo git python3 scip; do
  command -v "${tool}" >/dev/null || { echo "measure-edge-oracle.sh: ${tool} is required" >&2; exit 1; }
done
if ! bash "${REPO_ROOT}/scripts/utils/verification-lock.sh"; then
  exec bash "${REPO_ROOT}/scripts/utils/guard-benchmark.sh" \
    bash "${REPO_ROOT}/scripts/benchmarks/measure-edge-oracle.sh" "$@"
fi

language_of() {
  [[ "$1" == xcalygraph ]] && { echo rust; return; }
  [[ "$1" == graphify ]] && { echo python; return; }
  awk -F'\t' -v name="$1" '$1 == name { split($4, parts, ","); print parts[1] }' \
    "${REPO_ROOT}/scripts/benchmarks/corpora.tsv"
}

index_corpus() {
  local language="$1" root="$2" output="$3"
  case "${language}" in
    go) ( cd "${root}" && scip-go --output "${output}" ) ;;
    rust) bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
      ( cd "${root}" && rustup run "$(sed -n 's/^channel = "\(.*\)"/\1/p' "${REPO_ROOT}/rust-toolchain.toml")" \
              rust-analyzer scip . --output "${output}" ) ;;
    python) ( cd "${root}" && scip-python index . --output "${output}" ) ;;
    typescript|javascript)
      # Keep the pinned configuration. Resolve its development dependencies without running
      # lifecycle scripts or changing package.json/lockfiles; never disable a tsconfig.
      command -v bun >/dev/null || { echo "measure-edge-oracle.sh: bun is required for TypeScript dependencies" >&2; return 1; }
      # The indexer's inference scans node_modules and then disables JavaScript when it finds
      # a dependency's .d.ts. A JS corpus without a config needs its own explicit default.
      if [[ "${language}" == javascript && ! -e "${root}/tsconfig.json" && ! -L "${root}/tsconfig.json" ]]; then
        ( set -o noclobber; printf '%s\n' '{"compilerOptions":{"allowJs":true}}' > "${root}/tsconfig.json" )
      fi
      # A monorepo without a root config: one compiler project per app, each with its own
      # dependencies and aliases, under one index with repository-relative paths.
      if [[ "${language}" == typescript && ! -e "${root}/tsconfig.json" ]]; then
        local projects=() project
        while IFS= read -r project; do
          projects+=("${project%/tsconfig.json}")
          ( cd "${root}/${project%/tsconfig.json}" && bun install --no-save --ignore-scripts )
        done < <(cd "${root}" && find . -mindepth 2 -maxdepth 3 -name tsconfig.json -not -path '*/node_modules/*' | sed 's|^\./||' | LC_ALL=C sort)
        (( ${#projects[@]} > 0 )) || { echo "measure-edge-oracle.sh: no TypeScript project" >&2; return 1; }
        printf '%s\n' "${projects[@]}" > "${output%.scip}.projects"
        ( cd "${root}" && scip-typescript index "${projects[@]}" --output "${output}" )
        return
      fi
      ( cd "${root}" && bun install --no-save --ignore-scripts &&
          scip-typescript index --infer-tsconfig --output "${output}" ) ;;
    *) echo "measure-edge-oracle.sh: no indexer for ${language}" >&2; return 1 ;;
  esac
}

mkdir -p "${REPO_ROOT}/.cache/edge-oracle"
WORK="$(mktemp -d "${REPO_ROOT}/.cache/edge-oracle/run.XXXXXXXX")"
readonly WORK
echo "evidence: ${WORK}" >&2
printf 'corpus\tlanguage\tkind\tconfidence\tedges\tcorrect\tincorrect\tunknown\tprecision\ttarget\tagreement\n' > "${WORK}/summary.tsv"
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
  revision="$(git -C "${source_root}" rev-parse --verify HEAD)"
  git -C "${source_root}" archive --format=tar "${revision}" | tar -x -C "${copy}"
  printf '%s\n' "${revision}" > "${WORK}/${corpus}.revision"
  # Dump before installing compiler dependencies, so dependency directories never enter our graph.
  bash "${REPO_ROOT}/scripts/utils/check-build-space.sh"
  ( cd "${REPO_ROOT}" && XCALYGRAPH_DUMP_ROOT="${copy}" XCALYGRAPH_DUMP_FILE="${WORK}/${corpus}.edges.tsv" \
      cargo test -q --release --locked --bin xcalygraph resolve::tests::precision_tools::dump_edges -- --ignored --exact >/dev/null )
  index_corpus "${language}" "${copy}" "${WORK}/${corpus}.scip" > "${WORK}/${corpus}.index.log" 2>&1 || {
    echo "measure-edge-oracle.sh: ${corpus}: indexer failed; see ${WORK}/${corpus}.index.log" >&2
    exit 1
  }
  git --no-optional-locks -C "${source_root}" --work-tree="${copy}" diff --exit-code "${revision}" -- . \
    > "${WORK}/${corpus}.source.diff" || { echo "measure-edge-oracle.sh: ${corpus}: indexer changed pinned source" >&2; exit 1; }
  scip print --json "${WORK}/${corpus}.scip" > "${WORK}/${corpus}.scip.json"
  python3 -I - "${corpus}" "${language}" "${WORK}/${corpus}.scip.json" "${WORK}/${corpus}.edges.tsv" \
    "${WORK}/${corpus}.verdicts.tsv" >> "${WORK}/summary.tsv" <<'PYTHON'
import collections
import json
import os
import re
import sys

corpus, language, scip_path, edges_path, verdicts_path = sys.argv[1:]
index = json.load(open(scip_path))
# Several compiler projects may share a package name, so one symbol string would name a definition
# per app. A symbol belongs to the project of its document: no reference crosses projects unnoticed.
projects_path = scip_path.replace(".scip.json", ".projects")
if projects_path != scip_path and os.path.exists(projects_path):
    projects = open(projects_path, encoding="utf-8").read().split()
    for document in index["documents"]:
        project = next((p for p in projects if document["relative_path"].startswith(p + "/")), "")
        def scoped(symbol):
            return symbol if symbol.startswith("local ") else f"{project}|{symbol}"
        for occurrence in document.get("occurrences") or []:
            occurrence["symbol"] = scoped(occurrence["symbol"])
        for information in document.get("symbols") or []:
            information["symbol"] = scoped(information["symbol"])
            for relation in information.get("relationships") or []:
                relation["symbol"] = scoped(relation["symbol"])
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
# A reused compiler symbol cannot identify which physical target a reference reaches.
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

kinds = {information["symbol"]: information.get("kind")
         for document in index["documents"] for information in document.get("symbols") or []}
references = collections.defaultdict(set)
opaque_references = set()
for document in index["documents"]:
    local_definitions = {o["symbol"] for o in document["occurrences"] or []
                         if o.get("symbol_roles", 0) & DEFINITION}
    for occurrence in document["occurrences"] or []:
        symbol = occurrence["symbol"]
        if symbol.startswith("local ") and symbol not in local_definitions:
            # An unresolved import alias has occurrences but no definition or global identity.
            # Other resolved names on that line cannot establish which target it means.
            opaque_references.add((document["relative_path"], start_line(occurrence) + 1))
        if occurrence.get("symbol_roles", 0) & DEFINITION or symbol not in definitions:
            continue
        references[(document["relative_path"], start_line(occurrence) + 1)].update({symbol, owner(symbol)})

by_location = collections.defaultdict(set)
for symbol, locations in definitions.items():
    for path, line in locations:
        by_location[(path, descriptor_name(symbol))].add((symbol, line))

def targets_of(path, name, line):
    candidates = {symbol for symbol, defined in by_location.get((path, name), ()) if defined == line}
    return candidates if len(candidates) == 1 and all(len(definitions[symbol]) == 1 for symbol in candidates) else set()

def points(raw, typed=None):
    # SCIP's Go JSON encoding uses typed ranges and omits zero-valued coordinates.
    if not raw and typed:
        for kind, value in typed.items():
            if kind in ("SingleLineRange", "SingleLineEnclosingRange"):
                raw = [value.get("line", 0), value.get("start_character", 0), value.get("end_character", 0)]
            elif kind in ("MultiLineRange", "MultiLineEnclosingRange"):
                raw = [value.get("start_line", 0), value.get("start_character", 0),
                       value.get("end_line", 0), value.get("end_character", 0)]
    if not isinstance(raw, list) or len(raw) not in (3, 4):
        return None
    start = tuple(raw[:2])
    end = (raw[0], raw[2]) if len(raw) == 3 else tuple(raw[2:])
    return (start, end) if start <= end else None

# Definition enclosing ranges describe the source body. On a reference occurrence the
# enclosing range can describe its TARGET instead, so never use that as the caller.
source_scopes = collections.defaultdict(set)
owned_references = collections.defaultdict(set)
unknown_ownership = collections.defaultdict(set)
missing_scopes = set()
for document in index["documents"]:
    path = document["relative_path"]
    scopes = []
    document_kinds = {info["symbol"]: info.get("kind", 0) for info in document.get("symbols") or []}
    for occurrence in document["occurrences"] or []:
        symbol = occurrence["symbol"]
        # SCIP Function/Method/Constructor/Getter/Setter kinds include local symbols whose
        # descriptor contains only an ID. Missing accessor ranges must not bless their parent.
        callable_kind = document_kinds.get(symbol) in {9, 17, 18, 26, 45, 80}
        if not occurrence.get("symbol_roles", 0) & DEFINITION or not (
                callable_kind or symbol.endswith((").", "#", "/", "__init__:"))):
            continue
        body = points(occurrence.get("enclosing_range"), occurrence.get("TypedEnclosingRange"))
        declaration = points(occurrence.get("range"), occurrence.get("TypedRange"))
        if body is None or declaration is None:
            # A package/module declaration is not a callable body. Go and Python omit its
            # range; that does not invalidate the compiler's explicit function/type scopes.
            if callable_kind or symbol.endswith((").", "#")):
                missing_scopes.add(path)
            continue
        identity = (symbol, declaration[0])
        scopes.append((*body, identity))
        source_scopes[(path, descriptor_name(symbol), declaration[0][0] + 1)].add(identity)
    scopes.sort(key=lambda item: item[0])
    active, position = [], 0
    occurrences = sorted((o for o in document["occurrences"] or []
                          if points(o.get("range"), o.get("TypedRange"))),
                         key=lambda o: points(o.get("range"), o.get("TypedRange"))[0])
    for occurrence in occurrences:
        symbol = occurrence["symbol"]
        point = points(occurrence.get("range"), occurrence.get("TypedRange"))[0]
        active = [scope for scope in active if point < scope[1]]
        while position < len(scopes) and scopes[position][0] <= point:
            scope = scopes[position]
            if point < scope[1]:
                active.append(scope)
            position += 1
        if occurrence.get("symbol_roles", 0) & DEFINITION or symbol not in definitions:
            continue
        values = {symbol, owner(symbol)}
        location = (path, start_line(occurrence) + 1)
        if not active:
            if path in missing_scopes:
                unknown_ownership[location].update(values)
            else:
                owned_references[(*location, None)].update(values)
            continue
        start = max(scope[0] for scope in active)
        end = min(scope[1] for scope in active)
        owners = {scope[2] for scope in active if scope[:2] == (start, end)}
        if len(owners) != 1:
            unknown_ownership[location].update(values)
            continue
        enclosing = next(iter(owners))
        if enclosing[0].endswith(("/", "__init__:")):
            enclosing = None
        owned_references[(*location, enclosing)].update(values)

indexed = {document["relative_path"] for document in index["documents"]}
stats = collections.Counter()
with open(verdicts_path, "w") as verdicts:
    for row in open(edges_path, encoding="utf-8"):
        fields = row.rstrip("\n").split("\t")
        kind, confidence, source, line, target, target_line, name = fields[:7]
        # Only edges from files the compiler indexed (the corpus language) can be judged.
        if kind not in ("calls", "uses", "tests", "inherits", "implements") or source not in indexed:
            continue
        confidence = "exact" if confidence == "1" else "inferred"
        expected = targets_of(target, name, int(target_line))
        caller_known = False
        wrong_owner = False
        caller = None
        if len(fields) == 10:
            caller_line, caller_name, caller_kind = fields[7:]
            if caller_kind == "file":
                caller_known = source not in missing_scopes
            elif caller_kind == "symbol":
                candidates = source_scopes.get((source, caller_name, int(caller_line)), set())
                if len(candidates) == 1:
                    caller = next(iter(candidates))
                    caller_known = source not in missing_scopes
        if kind in ("inherits", "implements"):
            # The relationship belongs to the source type declared at the edge line.
            declared = targets_of(source, fields[8], int(fields[7])) if len(fields) == 10 else set()
            caller_known = bool(declared)
            oracle = ({parent for symbol in declared for parent in implements.get(symbol, ())
                       if parent in definitions} if len(declared) == 1 else set())
            dispatch = set()
        else:
            # Module and namespace symbols (`utils` -> file, scip-python's `pkg/__init__:` for a name
            # re-exported by the package) say nothing about the member called.
            oracle = {symbol for symbol in owned_references.get((source, int(line), caller), set())
                      if not symbol.endswith(("/", "__init__:"))}
            wrong_owner = bool(expected & references.get((source, int(line)), set())
                               and not expected & oracle
                               and not expected & unknown_ownership.get((source, int(line)), set()))
            if (kind in ("calls", "tests") and name not in {descriptor_name(symbol) for symbol in oracle}
                    and not any(symbol.endswith((").", "#")) for symbol in oracle)):
                # The compiler resolved no call on this line (only parameters or variables, e.g.
                # untyped pytest fixtures): it cannot judge the call edge.
                oracle = set()
            dispatch = {parent for symbol in expected for parent in implements.get(symbol, ())}
        names = {descriptor_name(symbol) for symbol in oracle}
        # A name read through a package's re-export reaches the compiler only as `pkg/__init__:`, so
        # the other symbols on that line cannot contradict the edge.
        reexported = any(symbol.endswith("__init__:") for symbol in owned_references.get((source, int(line), caller), set()))
        # Without any in-repo reference there, or without a compiler symbol for the target (local
        # functions, CommonJS export objects), the compiler cannot judge the edge.
        if not expected or not caller_known:
            verdict = "unknown"
        elif oracle & expected or oracle & dispatch:
            verdict = "correct"
        elif kind not in ("inherits", "implements") and (source, int(line)) in opaque_references:
            verdict = "unknown"
        elif expected & unknown_ownership.get((source, int(line)), set()):
            verdict = "unknown"
        elif reexported:
            verdict = "unknown"
        elif confidence == "inferred" and name in names:
            # A matching method name alone does not prove a relationship to this candidate.
            verdict = "unknown"
        elif wrong_owner:
            verdict = "incorrect"
        elif not oracle:
            verdict = "unknown"
        elif name in names and any(symbol.endswith(name + ".") and not symbol.endswith(")." )
                                   for symbol in oracle):
            # Field shorthand (`Struct { name }`): the compiler reports only the field.
            verdict = "unknown"
        else:
            verdict = "incorrect"
        # The compiler names this target on this line but cannot prove the caller's ownership:
        # target agreement, weaker than `correct`, stronger than silence.
        if verdict == "unknown" and expected & references.get((source, int(line)), set()):
            verdict = "target"
        stats[(kind, confidence, verdict)] += 1
        verdicts.write("\t".join([verdict, kind, confidence, source, line, target, target_line, name,
                                  ";".join(sorted(f"{path}:{defined}" for symbol in oracle
                                                  for path, defined in definitions[symbol])),
                                  *fields[7:]]) + "\n")

for kind, confidence in sorted({(k, c) for k, c, _ in stats}):
    correct, incorrect, unknown, target = (stats[(kind, confidence, v)]
                                           for v in ("correct", "incorrect", "unknown", "target"))
    judged = correct + incorrect
    precision = f"{100 * correct / judged:.1f}" if judged else "-"
    agreement = f"{100 * (correct + target) / (judged + target):.1f}" if judged + target else "-"
    print(f"{corpus}\t{language}\t{kind}\t{confidence}\t{correct + incorrect + unknown + target}\t{correct}\t{incorrect}\t{unknown}\t{precision}\t{target}\t{agreement}")

# Recall: compiler references from the corpus to in-repo callables and types, against edges on
# the same source file to the same target (any kind, any confidence).
edge_targets = collections.defaultdict(set)
for row in open(edges_path, encoding="utf-8"):
    kind, confidence, source, line, target, target_line, name = row.rstrip("\n").split("\t")[:7]
    for symbol in targets_of(target, name, int(target_line)):
        edge_targets[source].add(symbol)
        edge_targets[source].update(implements.get(symbol, ()))
pairs = set()
missed_lines = collections.defaultdict(set)
for (path, line), symbols in references.items():
    for symbol in symbols:
        if ACCESSOR.search(symbol) or owner(symbol) != symbol:
            continue  # property accessors are not calls; constructors count as their class
        group = "recall-calls" if symbol.endswith(").") else "recall-types" if symbol.endswith("#") else None
        # Enum variants (no graph nodes, docs/deferred.md) and associated types share `Type#Member#`;
        # a compiler kind that names something else keeps the symbol among the types.
        kind = kinds.get(symbol)
        if group == "recall-types" and language == "rust" and symbol.count("#") >= 2 and kind in (None, 0, 12, "EnumMember", "UnspecifiedKind"):
            group = "recall-variants"
        if group is not None:
            pair = (group, path, symbol)
            pairs.add(pair)
            if symbol not in edge_targets.get(path, ()):
                missed_lines[pair].add(line)
recall = collections.Counter()
with open(verdicts_path.replace(".verdicts.tsv", ".missed.tsv"), "w") as missed, \
     open(verdicts_path.replace(".verdicts.tsv", ".unknown.tsv"), "w") as unknown:
    for group, path, symbol in sorted(pairs):
        recall[(group, "total")] += 1
        locations = definitions[symbol]
        if len(locations) != 1 or not all(targets_of(target, descriptor_name(symbol), line) == {symbol}
                                           for target, line in locations):
            recall[(group, "unknown")] += 1
            unknown.write(f"{group}\t{path}\t{';'.join(f'{target}:{line}' for target, line in sorted(locations))}\t{symbol}\n")
        elif symbol in edge_targets.get(path, ()):
            recall[(group, "found")] += 1
        else:
            location = min(definitions[symbol])
            lines = ",".join(str(line) for line in sorted(missed_lines[(group, path, symbol)]))
            missed.write(f"{group}\t{path}\t{location[0]}:{location[1]}\t{symbol}\t{lines}\n")
for group in ("recall-calls", "recall-types", "recall-variants"):
    total, found, unknown = (recall[(group, value)] for value in ("total", "found", "unknown"))
    if total:
        judged = total - unknown
        share = f"{100 * found / judged:.1f}" if judged else "-"
        print(f"{corpus}\t{language}\t{group}\tall\t{total}\t{found}\t{judged - found}\t{unknown}\t{share}")
PYTHON
done
column -t -s $'\t' "${WORK}/summary.tsv"
( set -o noclobber; printf 'xcalygraph-cache-v1\n' > "${WORK}/.xcalygraph-disposable" )
