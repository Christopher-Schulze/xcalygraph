#!/usr/bin/env bash
# Exercise the real sealed evaluator, binary and Graphify on an exclusive temporary git corpus.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
python3 -I - "$REPO_ROOT" <<'PYTHON'
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

repo = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix="evaluation-labels.", dir=repo / ".cache") as temporary:
    work = Path(temporary)
    root = work / "repo"
    home = work / "home"
    root.mkdir()
    home.mkdir()
    environment = dict(os.environ, HOME=str(home))
    for relative in ("scripts/utils/prepare-corpus.sh", "scripts/benchmarks/evaluate-sealed.sh"):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(repo / relative, path)
        path.chmod(0o755)
    (root / ".gitignore").write_text(".cache/\n")
    (root / "main.go").write_text('package sample\nfunc Create() string { return "star" }\n\nfunc Other() {}\n')
    (root / "right.go").write_text("package sample\nfunc Delete() {}\n")
    for command in (["git", "init", "-q", "-b", "main", str(root)],
                    ["git", "-C", str(root), "add", "."],
                    ["git", "-C", str(root), "-c", "user.name=Test", "-c", "user.email=test@example.invalid",
                     "commit", "-qm", "fixture"]):
        subprocess.run(command, env=environment, check=True)
    helper = root / "scripts/utils/prepare-corpus.sh"
    result = subprocess.run(["bash", str(helper), str(root), str(root / ".cache/original"), "--graphify"],
                            env=environment, capture_output=True, text=True, check=True)
    source = Path(result.stdout.strip())
    metadata = json.loads((source.parent / "snapshot.json").read_text())
    # Force the real pinned-commit restore path without removing or mutating any evidence.
    frozen = root / ".cache/corpus-snapshots" / metadata["source_sha256"] / source.parent.name / "source"
    assert not frozen.exists()
    cases_dir = root / ".cache/cases"
    cases_dir.mkdir()

    def locator(path, line, name):
        data = (source / path).read_bytes()
        lines = data.decode().splitlines(keepends=True)
        end = min(line + 11, len(lines))
        return {"path": path, "line": line, "name": name,
                "file_sha256": hashlib.sha256(data).hexdigest(), "evidence_end": end,
                "evidence_sha256": hashlib.sha256("".join(lines[line-1:end]).encode()).hexdigest()}

    create = locator("main.go", 2, "Create")
    other = locator("main.go", 4, "Other")
    delete = locator("right.go", 2, "Delete")
    group = lambda loc: {"alternatives": [loc]}
    # Real answers to Create must not earn a definition hit for Other in the same file,
    # or a complete result for the two required source locations.
    cases = [
        {"id": "correct", "family": "symbol", "question": "Create", "requires": [group(create)]},
        {"id": "wrong-definition", "family": "behavior", "question": "Create", "requires": [group(other)]},
        {"id": "partial", "family": "multi", "question": "Create", "requires": [group(create), group(delete)]},
        {"id": "negative", "family": "negative", "question": "CreateAbsent", "requires": [],
         "absence": {"identifier": "CreateAbsent", "source_sha256": metadata["source_sha256"]}},
    ]
    case_data = "".join(json.dumps(case) + "\n" for case in cases).encode()
    (cases_dir / "fixture.jsonl").write_bytes(case_data)
    commit = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], env=environment,
                            capture_output=True, text=True, check=True).stdout.strip()
    manifest = {"schema": 1, "budgets": [600], "exposure": "isolated real-tool regression",
                "corpora": [{"name": "xcalygraph", "commit": commit,
                             "snapshot": str(frozen.relative_to(root)),
                             "source_sha256": metadata["source_sha256"], "upstream": metadata["upstream"],
                             "cases": "fixture.jsonl", "cases_sha256": hashlib.sha256(case_data).hexdigest(), "count": 4}]}
    manifest_path = cases_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest))
    runner = root / "scripts/benchmarks/evaluate-sealed.sh"
    arguments = ["bash", str(runner), str(manifest_path), "final", str(repo / "target/debug/xcalygraph")]
    run = subprocess.run(arguments, env=environment, capture_output=True, text=True)
    assert run.returncode == 0, run.stderr + run.stdout
    evidence = Path(run.stdout.splitlines()[-1].removeprefix("evidence: "))
    report = json.loads((evidence / "report.json").read_text())
    assert report["exposure"]["xcalygraph"]["first_evaluation"]
    for arm in ("find-text", "find-json"):
        scores = [json.loads(line) for line in (evidence / "xcalygraph" / (arm + "-600.scores.jsonl")).read_text().splitlines()]
        correct, wrong, partial, negative = scores
        assert correct["complete"] and correct["complete5"], scores
        assert wrong["file5"] and not wrong["complete"], scores
        assert partial["covered"] == 1 and partial["required"] == 2 and not partial["complete"], scores
        assert negative["false_positive"], scores
    repeat = subprocess.run(arguments, env=environment, capture_output=True, text=True)
    assert repeat.returncode != 0 and "already evaluated" in repeat.stderr
    assert len(list((root / ".cache/evaluation").glob("sealed.*/report.json"))) == 1
    print("PASS: pinned restore, text/JSON scoring, wrong-definition rejection, partial coverage, negative penalty and final exposure refusal")
PYTHON
