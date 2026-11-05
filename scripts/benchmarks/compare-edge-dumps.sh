#!/usr/bin/env bash
# Dump every resolved edge of the frozen corpora with the current source and report the difference
# to a baseline dump; the recall guard for extraction and resolver changes.
# Usage: scripts/benchmarks/compare-edge-dumps.sh ROOTS_TSV BASELINE_DIR OUTPUT_DIR
#   ROOTS_TSV: corpus, revision, source digest, root per line. OUTPUT_DIR must not exist; it receives
#   CORPUS.edges.tsv per corpus, delta.tsv (corpus, change, edge columns) and a summary on stdout.
#   An empty delta proves byte-identical edges. The exit code is 0 for any delta: the caller reviews.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
(( $# == 3 )) && [[ -f "$1" && -d "$2" && ! -e "$3" ]] || {
  echo "usage: compare-edge-dumps.sh ROOTS_TSV BASELINE_DIR NEW_OUTPUT_DIR" >&2
  exit 2
}
readonly ROOTS="$1" BASELINE="$2" OUTPUT="$3"

cd "${REPO_ROOT}"
bash scripts/utils/check-build-space.sh
# Facts do not depend on link-time optimization; the guard keeps its quick build.
producer="$(CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
  cargo test --locked --release --no-run --bin xcalygraph --message-format=json 2>/dev/null |
  jq -r 'select(.reason == "compiler-artifact" and .target.name == "xcalygraph" and .executable != null
    and (.target.kind | index("bin")) and .profile.test) | .executable' | tail -n 1)"
[[ -x "${producer}" ]] || { echo "compare-edge-dumps.sh: no test binary" >&2; exit 1; }

mkdir -p "${OUTPUT}"
export producer OUTPUT
# One dump per corpus; parallel per corpus, because most corpora are small.
# shellcheck disable=SC2016  # the inner shell expands its own arguments
cut -f1,4 "${ROOTS}" | xargs -P 4 -L 1 bash -c '
  XCALYGRAPH_DUMP_ROOT="$1" XCALYGRAPH_DUMP_FILE="${OUTPUT}/$0.edges.tsv" \
    "${producer}" --ignored --exact resolve::tests::precision_tools::dump_edges >/dev/null 2>&1 ||
    { echo "compare-edge-dumps.sh: dump failed for $0" >&2; exit 255; }'

python3 -I - "${ROOTS}" "${BASELINE}" "${OUTPUT}" <<'PY'
import collections
import pathlib
import sys

roots, baseline, output = sys.argv[1], pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3])
rows = []
identical = 0
corpora = [line.split("\t")[0] for line in open(roots, encoding="utf-8").read().splitlines()]


def edges(path):
    return collections.Counter(path.read_text(encoding="utf-8").splitlines())


for corpus in corpora:
    before = edges(baseline / f"{corpus}.edges.tsv")
    after = edges(output / f"{corpus}.edges.tsv")
    removed, added = before - after, after - before
    if not removed and not added:
        identical += 1
        continue
    print(corpus, "removed", removed.total(), "added", added.total())
    for change, lines in (("removed", removed), ("added", added)):
        rows.extend([corpus, change, line] for line in sorted(lines.elements()))
with (output / "delta.tsv").open("x", encoding="utf-8", newline="") as handle:
    for corpus, change, line in rows:
        handle.write(f"{corpus}\t{change}\t{line}\n")
print(f"{identical} of {len(corpora)} corpora identical, {len(rows)} changed edges")
PY
