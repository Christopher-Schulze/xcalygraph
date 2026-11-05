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
            caller = (16, "ambiguous_line") if line == 16 else (11, "entry")
            writer.writerow(("calls", confidence, path, line, target, target_line, name, *caller, "symbol"))
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
    assert exact[4:] == ["4", "1", "1", "2", "50.0", "0", "50.0"], exact
    assert inferred[4:] == ["2", "0", "0", "2", "-", "0", "-"], inferred

    caller_fixture = repo / "tests/fixtures/oracle/caller-identities"
    caller_index = caller_fixture / "compiler.json"
    for relative, expected_hash in json.loads(caller_index.read_text())["provenance"]["source_sha256"].items():
        assert hashlib.sha256((caller_fixture / relative).read_bytes()).hexdigest() == expected_hash, relative
    caller_edges = work / "callers.edges.tsv"
    caller_verdicts = work / "callers.verdicts.tsv"
    caller_cases = [
        # Same physical reference line, different real enclosing functions.
        ("src/lib.rs", 3, "src/lib.rs", 1, "first", 3, "left", "correct"),
        ("src/lib.rs", 3, "src/lib.rs", 2, "second", 3, "left", "incorrect"),
        ("src/lib.rs", 3, "src/lib.rs", 1, "first", 3, "right", "incorrect"),
        ("src/lib.rs", 3, "src/lib.rs", 2, "second", 3, "right", "correct"),
        # A nested function's call is not a call by its enclosing function.
        ("src/lib.rs", 4, "src/lib.rs", 1, "first", 4, "outer", "incorrect"),
        ("src/lib.rs", 4, "src/lib.rs", 1, "first", 4, "nested", "correct"),
        ("src/lib.rs", 4, "src/lib.rs", 4, "nested", 4, "outer", "correct"),
        # rust-analyzer shares the target identity between two test crates.
        ("tests/a.rs", 1, "tests/b.rs", 3, "target", 1, "repeated", "unknown"),
        ("src/lib.rs", 3, "src/lib.rs", 1, "first", 3, "missing", "target"),
    ]
    with caller_edges.open("x", newline="") as output:
        writer = csv.writer(output, delimiter="\t", lineterminator="\n")
        for path, line, target, target_line, name, caller_line, caller, _ in caller_cases:
            writer.writerow(("calls", "1", path, line, target, target_line, name, caller_line, caller, "symbol"))
        # Historical seven-column evidence cannot prove the identity of the caller.
        writer.writerow(("calls", "1", "src/lib.rs", 3, "src/lib.rs", 1, "first"))
    caller_result = subprocess.run(
        [sys.executable, "-I", "-", "callers", "rust", str(caller_index), str(caller_edges), str(caller_verdicts)],
        input=program, text=True, capture_output=True, env=dict(os.environ, HOME=str(home)), check=True,
    )
    actual = [row[0] for row in csv.reader(caller_verdicts.open(), delimiter="\t")]
    expected = [case[-1] for case in caller_cases] + ["target"]
    assert actual == expected, f"caller oracle verdicts: {actual}; expected {expected}"
    recall = next(row for row in csv.reader(caller_result.stdout.splitlines(), delimiter="\t") if row[2] == "recall-calls")
    assert recall[4:] == ["5", "3", "0", "2", "100.0"], recall
    for fixture_name, language, kind, cases in [
        ("accessor-identities", "typescript", "calls", [("index.ts", 3, "index.ts", 1, "first", 2, "Box", "target")]),
        ("local-accessor-identities", "typescript", "calls", [
            ("index.ts", 2, "index.ts", 1, "first", 2, "outer", "target"),
            ("plain.ts", 2, "plain.ts", 1, "first", 2, "caller", "correct"),
        ]),
        ("go-caller-identities", "go", "calls", [
            ("caller.go", 4, "caller.go", 2, "First", 4, "Left", "correct"),
            ("caller.go", 4, "caller.go", 3, "Second", 4, "Left", "incorrect"),
            ("caller.go", 4, "caller.go", 2, "First", 4, "Right", "incorrect"),
            ("caller.go", 4, "caller.go", 3, "Second", 4, "Right", "correct"),
            ("caller.go", 6, "caller.go", 2, "First", 5, "Multi", "correct"),
        ]),
        ("opaque-import-identities", "typescript", "uses", [
            ("index.ts", 3, "definitions.ts", 1, "Target", 3, "caller", "unknown"),
            ("index.ts", 4, "definitions.ts", 1, "Target", 4, "wrong", "incorrect"),
            ("index.ts", 4, "index.ts", 2, "Other", 4, "wrong", "correct"),
        ]),
    ]:
        fixture = repo / "tests/fixtures/oracle" / fixture_name
        compiler = fixture / "compiler.json"
        for relative, expected_hash in json.loads(compiler.read_text())["provenance"]["source_sha256"].items():
            assert hashlib.sha256((fixture / relative).read_bytes()).hexdigest() == expected_hash, relative
        edges = work / (fixture_name + ".edges.tsv")
        verdicts = work / (fixture_name + ".verdicts.tsv")
        with edges.open("x", newline="") as output:
            writer = csv.writer(output, delimiter="\t", lineterminator="\n")
            for path, line, target, target_line, name, caller_line, caller, _ in cases:
                writer.writerow((kind, "1", path, line, target, target_line, name, caller_line, caller, "symbol"))
        subprocess.run(
            [sys.executable, "-I", "-", fixture_name, language, str(compiler), str(edges), str(verdicts)],
            input=program, text=True, capture_output=True, env=dict(os.environ, HOME=str(home)), check=True,
        )
        actual = [row[0] for row in csv.reader(verdicts.open(), delimiter="\t")]
        assert actual == [case[-1] for case in cases], f"{fixture_name}: {actual}"
print("PASS: real compiler identities prove targets and callers, reject shared-line/nested calls, and qualify ambiguous recall")
PYTHON
