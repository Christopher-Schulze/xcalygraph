#!/usr/bin/env bash
# Compare real recorded rust-analyzer identities against nearby and unrelated edge targets.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && /bin/pwd -P)"
readonly REPO_ROOT
python3 -I - "${REPO_ROOT}" <<'PYTHON'
import csv
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
fixture = repo / "tests/fixtures/oracle/receiver-identities"
compiler = fixture / "compiler.json"
index = json.loads(compiler.read_text())
for relative, expected in index["provenance"]["source_sha256"].items():
    actual = hashlib.sha256((fixture / relative).read_bytes()).hexdigest()
    assert actual == expected, f"compiler fixture is stale: {relative}"

# Exercise the comparator used by the measurement script, with no indexer needed in the gate.
script = repo / "scripts/benchmarks/measure-edge-oracle.sh"
source = script.read_text()
delimiter = "<<'PYTHON'\n"
assert source.count(delimiter) == 1, "oracle comparator boundary changed"
program, boundary, _ = source.split(delimiter, 1)[1].partition("\nPYTHON\n")
assert boundary, "oracle comparator has no closing delimiter"
compile(program, str(script), "exec")

cache = repo / ".cache"
assert not cache.is_symlink(), "test cache must not be a symlink"
cache.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix="edge-oracle-regression.", dir=cache) as temporary:
    work = Path(temporary)
    home = work / "home"
    home.mkdir()
    edges = work / "fixture.edges.tsv"
    verdicts = work / "fixture.verdicts.tsv"
    cases = [
        # The real call at line 11 resolves to Value::finish at line 9, not Wrapper::finish.
        ("1", "src/lib.rs", 11, "src/lib.rs", 6, "finish", "incorrect"),
        ("0", "src/lib.rs", 11, "src/lib.rs", 6, "finish", "unknown"),
        ("1", "src/lib.rs", 11, "src/lib.rs", 9, "finish", "correct"),
        # An unrelated same-named method is not compiler evidence for inferred dispatch.
        ("0", "src/lib.rs", 11, "src/unrelated.rs", 3, "finish", "unknown"),
        # Two definitions sharing a line cannot be identified by the dumped location.
        ("1", "src/lib.rs", 16, "src/lib.rs", 15, "clash", "unknown"),
        ("1", "src/lib.rs", 11, "src/lib.rs", 7, "finish", "unknown"),
    ]
    with edges.open("x", newline="") as output:
        writer = csv.writer(output, delimiter="\t", lineterminator="\n")
        for confidence, path, line, target, target_line, name, _ in cases:
            writer.writerow(("calls", confidence, path, line, target, target_line, name))
    result = subprocess.run(
        [sys.executable, "-I", "-", "fixture", "rust", str(compiler), str(edges), str(verdicts)],
        input=program, text=True, capture_output=True, env=dict(os.environ, HOME=str(home)),
        check=True,
    )
    actual = [row[0] for row in csv.reader(verdicts.open(), delimiter="\t")]
    expected = [case[-1] for case in cases]
    assert actual == expected, f"oracle verdicts: {actual}; expected {expected}"
    summaries = list(csv.reader(result.stdout.splitlines(), delimiter="\t"))
    exact = next(row for row in summaries if row[2:4] == ["calls", "exact"])
    inferred = next(row for row in summaries if row[2:4] == ["calls", "inferred"])
    assert exact[4:] == ["4", "1", "1", "2", "50.0"], exact
    assert inferred[4:] == ["2", "0", "0", "2", "-"], inferred
print("PASS: real compiler identities reject nearby targets and leave ambiguous or name-only evidence unknown")
PYTHON
